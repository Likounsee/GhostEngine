//! Backend-independent vector paths and deterministic curve flattening.

use crate::{Color, Framebuffer, Rect, Transform2D, Vector};

#[derive(Debug, Clone, Copy, PartialEq)]
enum Command {
    MoveTo(Vector),
    LineTo(Vector),
    QuadTo(Vector, Vector),
    CubicTo(Vector, Vector, Vector),
    Close,
}

#[derive(Debug, Clone, Copy)]
struct FillEdge {
    min_y: f32,
    max_y: f32,
    min_x: f64,
    inverse_slope: f64,
    winding: i32,
}

impl FillEdge {
    fn new(a: Vector, b: Vector) -> Option<Self> {
        // Only exactly horizontal edges are irrelevant to scan conversion.
        // An epsilon test incorrectly discards very thin but valid contours
        // that still cross a pixel-center sample.
        if a.y == b.y {
            return None;
        }
        let (lower, upper) = if a.y < b.y { (a, b) } else { (b, a) };
        Some(Self {
            min_y: lower.y,
            max_y: upper.y,
            // Finite f32 endpoints can still overflow during f32 subtraction
            // (for example, -f32::MAX to f32::MAX). Keep interpolation in f64.
            min_x: f64::from(lower.x),
            inverse_slope: (f64::from(upper.x) - f64::from(lower.x))
                / (f64::from(upper.y) - f64::from(lower.y)),
            winding: if a.y < b.y { 1 } else { -1 },
        })
    }

    fn x_at(self, sample_y: f32) -> f32 {
        (self.min_x + (f64::from(sample_y) - f64::from(self.min_y)) * self.inverse_slope) as f32
    }

    fn crosses(self, sample_y: f32) -> bool {
        self.min_y <= sample_y && sample_y < self.max_y
    }
}

/// Adds horizontal 4x supersampling coverage for one scanline interval.
///
/// Fully covered pixels contribute four samples without testing each sample
/// individually. Only the two boundary regions need per-sample comparisons.
fn accumulate_horizontal_coverage(
    coverage: &mut [u8],
    min_x: u32,
    max_x: u32,
    previous_x: f32,
    x: f32,
) {
    if previous_x >= x || min_x >= max_x {
        return;
    }

    let span_start = ((previous_x - 0.875).floor() as i64)
        .max(i64::from(min_x))
        .min(i64::from(max_x)) as u32;
    let span_end = ((x - 0.125).ceil() as i64)
        .max(i64::from(min_x))
        .min(i64::from(max_x)) as u32;
    if span_start >= span_end {
        return;
    }

    let full_start = ((previous_x - 0.125).ceil() as i64)
        .max(i64::from(span_start))
        .min(i64::from(span_end)) as u32;
    let full_end = ((x - 0.875).ceil() as i64)
        .max(i64::from(span_start))
        .min(i64::from(span_end)) as u32;

    fn accumulate_partial(
        coverage: &mut [u8],
        min_x: u32,
        start: u32,
        end: u32,
        previous_x: f32,
        x: f32,
    ) {
        const OFFSETS: [f32; 4] = [0.125, 0.375, 0.625, 0.875];
        for pixel_x in start..end {
            let sample_count = OFFSETS
                .iter()
                .filter(|&&offset_x| {
                    let sample_x = pixel_x as f32 + offset_x;
                    previous_x <= sample_x && sample_x < x
                })
                .count() as u8;
            coverage[(pixel_x - min_x) as usize] += sample_count;
        }
    }

    if full_start <= full_end {
        accumulate_partial(coverage, min_x, span_start, full_start, previous_x, x);
        for pixel_x in full_start..full_end {
            coverage[(pixel_x - min_x) as usize] += 4;
        }
        accumulate_partial(coverage, min_x, full_end, span_end, previous_x, x);
    } else {
        accumulate_partial(coverage, min_x, span_start, span_end, previous_x, x);
    }
}

/// Collects closed contours with fallible allocation before rasterization.
fn try_collect_contours(flattened: &FlattenedPath) -> Result<Vec<&[Vector]>, PathError> {
    let count = flattened
        .subpaths
        .iter()
        .filter(|subpath| subpath.closed && subpath.points.len() >= 3)
        .count();
    let mut contours = Vec::new();
    contours
        .try_reserve_exact(count)
        .map_err(|_| PathError::AllocationFailed)?;
    contours.extend(
        flattened
            .subpaths
            .iter()
            .filter(|subpath| subpath.closed && subpath.points.len() >= 3)
            .map(|subpath| subpath.points.as_slice()),
    );
    Ok(contours)
}

/// Builds scan-conversion edges without infallible Vec growth.
fn try_collect_fill_edges(contours: &[&[Vector]]) -> Result<Vec<FillEdge>, PathError> {
    let capacity = contours.iter().try_fold(0_usize, |total, contour| {
        total
            .checked_add(contour.len())
            .ok_or(PathError::AllocationFailed)
    })?;
    let mut edges = Vec::new();
    edges
        .try_reserve_exact(capacity)
        .map_err(|_| PathError::AllocationFailed)?;
    for contour in contours {
        for index in 0..contour.len() {
            if let Some(edge) = FillEdge::new(contour[index], contour[(index + 1) % contour.len()])
            {
                edges.push(edge);
            }
        }
    }
    Ok(edges)
}

fn try_zeroed_coverage(width: usize) -> Result<Vec<u8>, PathError> {
    let mut coverage = Vec::new();
    coverage
        .try_reserve_exact(width)
        .map_err(|_| PathError::AllocationFailed)?;
    coverage.resize(width, 0);
    Ok(coverage)
}

/// A vector path made of line, quadratic Bézier and cubic Bézier segments.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Path {
    commands: Vec<Command>,
}

/// A flattened contour. Closed contours include no duplicated final point.
#[derive(Debug, Clone, PartialEq)]
pub struct FlattenedSubpath {
    pub points: Vec<Vector>,
    pub closed: bool,
}

/// Flattened path geometry ready for rasterization.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FlattenedPath {
    pub subpaths: Vec<FlattenedSubpath>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathError {
    MissingCurrentPoint,
    InvalidTolerance,
    NonFiniteCoordinate,
    InvalidTransform,
    AllocationFailed,
}

impl Path {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            commands: Vec::new(),
        }
    }

    /// Returns the number of path commands that fit without growing storage.
    #[must_use]
    pub fn command_capacity(&self) -> usize {
        self.commands.capacity()
    }

    /// Returns logical bytes held by the path command buffer.
    #[must_use]
    pub fn allocated_bytes(&self) -> usize {
        self.commands
            .capacity()
            .saturating_mul(core::mem::size_of::<Command>())
    }

    /// Removes all path commands while retaining their backing allocation.
    pub fn clear(&mut self) {
        self.commands.clear();
    }

    /// Drops the path command allocation.
    pub fn release(&mut self) {
        self.clear();
        self.commands.shrink_to_fit();
    }

    /// Releases path command storage when it exceeds the budget.
    pub fn trim_to_budget(&mut self, budget_bytes: usize) -> usize {
        let before = self.allocated_bytes();
        if before <= budget_bytes {
            return 0;
        }
        self.release();
        before.saturating_sub(self.allocated_bytes())
    }

    /// Reserves capacity for additional path commands without changing the path.
    pub fn try_reserve_commands(&mut self, additional: usize) -> Result<(), PathError> {
        self.commands
            .try_reserve(additional)
            .map_err(|_| PathError::AllocationFailed)
    }

    /// Appends a move command, returning an error if storage cannot be reserved.
    pub fn try_move_to(&mut self, point: Vector) -> Result<&mut Self, PathError> {
        self.try_reserve_commands(1)?;
        self.commands.push(Command::MoveTo(point));
        Ok(self)
    }

    /// Appends a line command, returning an error if storage cannot be reserved.
    pub fn try_line_to(&mut self, point: Vector) -> Result<&mut Self, PathError> {
        self.try_reserve_commands(1)?;
        self.commands.push(Command::LineTo(point));
        Ok(self)
    }

    /// Appends a quadratic Bézier command with fallible storage reservation.
    pub fn try_quadratic_to(
        &mut self,
        control: Vector,
        end: Vector,
    ) -> Result<&mut Self, PathError> {
        self.try_reserve_commands(1)?;
        self.commands.push(Command::QuadTo(control, end));
        Ok(self)
    }

    /// Appends a cubic Bézier command with fallible storage reservation.
    pub fn try_cubic_to(
        &mut self,
        c1: Vector,
        c2: Vector,
        end: Vector,
    ) -> Result<&mut Self, PathError> {
        self.try_reserve_commands(1)?;
        self.commands.push(Command::CubicTo(c1, c2, end));
        Ok(self)
    }

    /// Appends a close command, returning an error if storage cannot be reserved.
    pub fn try_close(&mut self) -> Result<&mut Self, PathError> {
        self.try_reserve_commands(1)?;
        self.commands.push(Command::Close);
        Ok(self)
    }

    /// Appends a move command. Panics only if command storage cannot be allocated.
    pub fn move_to(&mut self, point: Vector) -> &mut Self {
        self.try_move_to(point)
            .expect("path command allocation failed")
    }

    /// Appends a line command. Panics only if command storage cannot be allocated.
    pub fn line_to(&mut self, point: Vector) -> &mut Self {
        self.try_line_to(point)
            .expect("path command allocation failed")
    }

    /// Appends a quadratic Bézier command.
    pub fn quadratic_to(&mut self, control: Vector, end: Vector) -> &mut Self {
        self.try_quadratic_to(control, end)
            .expect("path command allocation failed")
    }

    /// Appends a cubic Bézier command.
    pub fn cubic_to(&mut self, c1: Vector, c2: Vector, end: Vector) -> &mut Self {
        self.try_cubic_to(c1, c2, end)
            .expect("path command allocation failed")
    }

    /// Closes the current contour.
    pub fn close(&mut self) -> &mut Self {
        self.try_close().expect("path command allocation failed")
    }

    /// Flattens curves until their control points are within tolerance of the chord.
    /// Tolerance is measured in the path's local coordinate space.
    ///
    /// All geometry allocations are fallible and reported as PathError::AllocationFailed.
    pub fn flatten(&self, tolerance: f32) -> Result<FlattenedPath, PathError> {
        self.try_flatten(tolerance)
    }

    /// Fallibly flattens this path without panicking on geometry allocation failure.
    pub fn try_flatten(&self, tolerance: f32) -> Result<FlattenedPath, PathError> {
        if !tolerance.is_finite() || tolerance <= 0.0 {
            return Err(PathError::InvalidTolerance);
        }
        let tolerance_squared = f64::from(tolerance) * f64::from(tolerance);
        let mut result = FlattenedPath::default();
        result
            .subpaths
            .try_reserve(self.commands.len().min(16))
            .map_err(|_| PathError::AllocationFailed)?;
        let mut current: Option<usize> = None;
        // After Close, the next drawing command starts a new contour at the
        // closed contour's initial point, matching conventional path semantics.
        let mut closed_start: Option<Vector> = None;

        for command in &self.commands {
            match *command {
                Command::MoveTo(point) => {
                    validate(point)?;
                    let mut points = Vec::new();
                    points
                        .try_reserve_exact(1)
                        .map_err(|_| PathError::AllocationFailed)?;
                    points.push(point);
                    result
                        .subpaths
                        .try_reserve(1)
                        .map_err(|_| PathError::AllocationFailed)?;
                    result.subpaths.push(FlattenedSubpath {
                        points,
                        closed: false,
                    });
                    current = Some(result.subpaths.len() - 1);
                    closed_start = None;
                }
                Command::LineTo(point) => {
                    validate(point)?;
                    let index =
                        ensure_current_subpath(&mut result, &mut current, &mut closed_start)?;
                    push_point(&mut result.subpaths[index].points, point)?;
                }
                Command::QuadTo(control, end) => {
                    validate(control)?;
                    validate(end)?;
                    let index =
                        ensure_current_subpath(&mut result, &mut current, &mut closed_start)?;
                    let contour = &mut result.subpaths[index];
                    let start = *contour
                        .points
                        .last()
                        .ok_or(PathError::MissingCurrentPoint)?;
                    flatten_quad(
                        start,
                        control,
                        end,
                        tolerance_squared,
                        0,
                        &mut contour.points,
                    )?;
                }
                Command::CubicTo(c1, c2, end) => {
                    validate(c1)?;
                    validate(c2)?;
                    validate(end)?;
                    let index =
                        ensure_current_subpath(&mut result, &mut current, &mut closed_start)?;
                    let contour = &mut result.subpaths[index];
                    let start = *contour
                        .points
                        .last()
                        .ok_or(PathError::MissingCurrentPoint)?;
                    flatten_cubic(
                        start,
                        c1,
                        c2,
                        end,
                        tolerance_squared,
                        0,
                        &mut contour.points,
                    )?;
                }
                Command::Close => {
                    let index = current.ok_or(PathError::MissingCurrentPoint)?;
                    let contour = &mut result.subpaths[index];
                    contour.closed = true;
                    closed_start = contour.points.first().copied();
                    current = None;
                }
            }
        }
        Ok(result)
    }
}

fn validate(p: Vector) -> Result<(), PathError> {
    if p.x.is_finite() && p.y.is_finite() {
        Ok(())
    } else {
        Err(PathError::NonFiniteCoordinate)
    }
}
// Use f64 for intermediate geometry: finite f32 coordinates can still
// overflow when subtracted, squared, or averaged in f32.
fn distance_to_segment_squared(p: Vector, a: Vector, b: Vector) -> f64 {
    let px = f64::from(p.x);
    let py = f64::from(p.y);
    let ax = f64::from(a.x);
    let ay = f64::from(a.y);
    let dx = f64::from(b.x) - ax;
    let dy = f64::from(b.y) - ay;
    let length_squared = dx * dx + dy * dy;
    if length_squared <= f64::EPSILON {
        let offset_x = px - ax;
        let offset_y = py - ay;
        return offset_x * offset_x + offset_y * offset_y;
    }

    let projection = (((px - ax) * dx + (py - ay) * dy) / length_squared).clamp(0.0, 1.0);
    let offset_x = px - (ax + projection * dx);
    let offset_y = py - (ay + projection * dy);
    offset_x * offset_x + offset_y * offset_y
}
fn midpoint(a: Vector, b: Vector) -> Vector {
    Vector::new(
        ((f64::from(a.x) + f64::from(b.x)) * 0.5) as f32,
        ((f64::from(a.y) + f64::from(b.y)) * 0.5) as f32,
    )
}

fn ensure_current_subpath(
    result: &mut FlattenedPath,
    current: &mut Option<usize>,
    closed_start: &mut Option<Vector>,
) -> Result<usize, PathError> {
    if let Some(index) = *current {
        return Ok(index);
    }

    let start = closed_start.take().ok_or(PathError::MissingCurrentPoint)?;
    let mut points = Vec::new();
    points
        .try_reserve_exact(1)
        .map_err(|_| PathError::AllocationFailed)?;
    points.push(start);
    result
        .subpaths
        .try_reserve(1)
        .map_err(|_| PathError::AllocationFailed)?;
    result.subpaths.push(FlattenedSubpath {
        points,
        closed: false,
    });
    let index = result.subpaths.len() - 1;
    *current = Some(index);
    Ok(index)
}

fn push_point(out: &mut Vec<Vector>, point: Vector) -> Result<(), PathError> {
    out.try_reserve(1)
        .map_err(|_| PathError::AllocationFailed)?;
    out.push(point);
    Ok(())
}

fn flatten_quad(
    a: Vector,
    c: Vector,
    b: Vector,
    tolerance_squared: f64,
    depth: u8,
    out: &mut Vec<Vector>,
) -> Result<(), PathError> {
    if depth >= 16 || distance_to_segment_squared(c, a, b) <= tolerance_squared {
        return push_point(out, b);
    }
    let ac = midpoint(a, c);
    let cb = midpoint(c, b);
    let m = midpoint(ac, cb);
    flatten_quad(a, ac, m, tolerance_squared, depth + 1, out)?;
    flatten_quad(m, cb, b, tolerance_squared, depth + 1, out)
}

fn flatten_cubic(
    a: Vector,
    c1: Vector,
    c2: Vector,
    b: Vector,
    tolerance_squared: f64,
    depth: u8,
    out: &mut Vec<Vector>,
) -> Result<(), PathError> {
    if depth >= 16
        || distance_to_segment_squared(c1, a, b).max(distance_to_segment_squared(c2, a, b))
            <= tolerance_squared
    {
        return push_point(out, b);
    }
    let a1 = midpoint(a, c1);
    let c = midpoint(c1, c2);
    let b2 = midpoint(c2, b);
    let a2 = midpoint(a1, c);
    let b1 = midpoint(c, b2);
    let m = midpoint(a2, b1);
    flatten_cubic(a, a1, a2, m, tolerance_squared, depth + 1, out)?;
    flatten_cubic(m, b1, b2, b, tolerance_squared, depth + 1, out)
}

/// Transforms flattened geometry without changing the source path.
impl FlattenedPath {
    /// Transforms flattened geometry while reporting allocation failures.
    pub fn try_transformed(&self, transform: Transform2D) -> Result<Self, PathError> {
        if !transform.is_finite() {
            return Err(PathError::InvalidTransform);
        }

        let mut result = FlattenedPath::default();
        result
            .subpaths
            .try_reserve_exact(self.subpaths.len())
            .map_err(|_| PathError::AllocationFailed)?;

        for subpath in &self.subpaths {
            let mut points = Vec::new();
            points
                .try_reserve_exact(subpath.points.len())
                .map_err(|_| PathError::AllocationFailed)?;
            for &point in &subpath.points {
                let transformed = transform.transform_point(point);
                validate(transformed)?;
                points.push(transformed);
            }
            result.subpaths.push(FlattenedSubpath {
                points,
                closed: subpath.closed,
            });
        }
        Ok(result)
    }

    /// Transforms flattened geometry without changing the source path.
    ///
    /// Use try_transformed when allocation errors must be handled.
    pub fn transformed(&self, transform: Transform2D) -> Result<Self, PathError> {
        self.try_transformed(transform)
    }
}

/// Fills all closed contours using the even-odd rule. Curves are flattened first.
pub fn fill_path_even_odd(
    target: &mut Framebuffer,
    path: &Path,
    color: Color,
    tolerance: f32,
) -> Result<(), PathError> {
    fill_transformed_path_even_odd(target, path, Transform2D::IDENTITY, color, tolerance)
}

/// Fills a path using the even-odd rule, restricted to a framebuffer viewport.
pub fn fill_path_even_odd_clipped(
    target: &mut Framebuffer,
    path: &Path,
    color: Color,
    tolerance: f32,
    clip: Rect,
) -> Result<(), PathError> {
    fill_transformed_path_even_odd_clipped(
        target,
        path,
        Transform2D::IDENTITY,
        color,
        tolerance,
        clip,
    )
}

/// Flattens, transforms and fills a path in one operation.
/// The transform is applied before pixel-center scan conversion, so rotation,
/// scaling, skew and translation affect the actual rasterized geometry.
pub fn fill_transformed_path_even_odd(
    target: &mut Framebuffer,
    path: &Path,
    transform: Transform2D,
    color: Color,
    tolerance: f32,
) -> Result<(), PathError> {
    let clip = Rect::new(0, 0, target.width(), target.height());
    fill_transformed_path_even_odd_clipped(target, path, transform, color, tolerance, clip)
}

/// Flattens, transforms and fills a path while restricting writes to a viewport.
///
/// Geometry is still validated when the clip is empty. The viewport is expressed
/// in framebuffer coordinates and intersected with the framebuffer before rasterization.
pub fn fill_transformed_path_even_odd_clipped(
    target: &mut Framebuffer,
    path: &Path,
    transform: Transform2D,
    color: Color,
    tolerance: f32,
    clip: Rect,
) -> Result<(), PathError> {
    let flattened = path.flatten(tolerance)?.try_transformed(transform)?;
    let Some(clip) = clip.intersection(Rect::new(0, 0, target.width(), target.height())) else {
        return Ok(());
    };
    if target.width() == 0 || target.height() == 0 {
        return Ok(());
    }
    let clip_right = clip.x.saturating_add(clip.width);
    let clip_bottom = clip.y.saturating_add(clip.height);
    let contours = try_collect_contours(&flattened)?;
    if contours.is_empty() {
        return Ok(());
    }

    let edges = try_collect_fill_edges(&contours)?;
    if edges.is_empty() {
        return Ok(());
    }

    let min_y = edges
        .iter()
        .map(|edge| edge.min_y)
        .fold(f32::INFINITY, f32::min)
        .floor()
        .max(clip.y as f32)
        .min(clip_bottom as f32) as u32;
    let max_y = edges
        .iter()
        .map(|edge| edge.max_y)
        .fold(f32::NEG_INFINITY, f32::max)
        .ceil()
        .max(clip.y as f32)
        .min(clip_bottom as f32) as u32;
    // Keep only edges that cross the current scanline. Sorting once by
    // their lower endpoint avoids revisiting every edge on sparse rows.
    let mut edges = edges;
    edges.sort_unstable_by(|left, right| left.min_y.total_cmp(&right.min_y));
    let mut active_edges = Vec::new();
    active_edges
        .try_reserve_exact(edges.len())
        .map_err(|_| PathError::AllocationFailed)?;
    let mut next_edge = 0;
    let mut intersections = Vec::new();
    intersections
        .try_reserve_exact(edges.len())
        .map_err(|_| PathError::AllocationFailed)?;
    for y in min_y..max_y {
        intersections.clear();
        let scan = y as f32 + 0.5;
        while next_edge < edges.len() && edges[next_edge].min_y <= scan {
            let edge = edges[next_edge];
            if edge.crosses(scan) {
                active_edges.push(edge);
            }
            next_edge += 1;
        }
        active_edges.retain(|edge| edge.crosses(scan));
        for &edge in &active_edges {
            intersections.push(edge.x_at(scan));
        }
        intersections.sort_unstable_by(f32::total_cmp);
        let Some(row) = target.row_mut(y) else {
            continue;
        };
        for pair in intersections.chunks_exact(2) {
            let left = ((pair[0] - 0.5).ceil().max(clip.x as f32) as u32).min(clip_right);
            let right = ((pair[1] - 0.5).ceil().max(clip.x as f32) as u32).min(clip_right);
            if left < right {
                row[left as usize..right as usize].fill(color);
            }
        }
    }
    Ok(())
}

/// Fills all closed contours using the non-zero winding rule.
pub fn fill_path_non_zero(
    target: &mut Framebuffer,
    path: &Path,
    color: Color,
    tolerance: f32,
) -> Result<(), PathError> {
    fill_transformed_path_non_zero(target, path, Transform2D::IDENTITY, color, tolerance)
}

/// Fills a path with the non-zero winding rule inside a framebuffer-space clip.
pub fn fill_path_non_zero_clipped(
    target: &mut Framebuffer,
    path: &Path,
    color: Color,
    tolerance: f32,
    clip: Rect,
) -> Result<(), PathError> {
    fill_transformed_path_non_zero_clipped(
        target,
        path,
        Transform2D::IDENTITY,
        color,
        tolerance,
        clip,
    )
}

/// Flattens, transforms and fills a path using the non-zero winding rule.
pub fn fill_transformed_path_non_zero(
    target: &mut Framebuffer,
    path: &Path,
    transform: Transform2D,
    color: Color,
    tolerance: f32,
) -> Result<(), PathError> {
    fill_transformed_path_non_zero_clipped(
        target,
        path,
        transform,
        color,
        tolerance,
        Rect::new(0, 0, target.width(), target.height()),
    )
}

/// Non-zero winding fill restricted to a framebuffer-space viewport.
pub fn fill_transformed_path_non_zero_clipped(
    target: &mut Framebuffer,
    path: &Path,
    transform: Transform2D,
    color: Color,
    tolerance: f32,
    clip: Rect,
) -> Result<(), PathError> {
    let flattened = path.flatten(tolerance)?.try_transformed(transform)?;
    let Some(clip) = clip.intersection(Rect::new(0, 0, target.width(), target.height())) else {
        return Ok(());
    };
    if target.width() == 0 || target.height() == 0 {
        return Ok(());
    }

    let clip_right = clip.x.saturating_add(clip.width);
    let clip_bottom = clip.y.saturating_add(clip.height);
    let contours = try_collect_contours(&flattened)?;
    let mut edges = try_collect_fill_edges(&contours)?;
    if edges.is_empty() {
        return Ok(());
    }

    let min_y = edges
        .iter()
        .map(|edge| edge.min_y)
        .fold(f32::INFINITY, f32::min)
        .floor()
        .max(clip.y as f32)
        .min(clip_bottom as f32) as u32;
    let max_y = edges
        .iter()
        .map(|edge| edge.max_y)
        .fold(f32::NEG_INFINITY, f32::max)
        .ceil()
        .max(clip.y as f32)
        .min(clip_bottom as f32) as u32;
    edges.sort_unstable_by(|left, right| left.min_y.total_cmp(&right.min_y));

    let mut active_edges = Vec::new();
    active_edges
        .try_reserve_exact(edges.len())
        .map_err(|_| PathError::AllocationFailed)?;
    let mut intersections: Vec<(f32, i32)> = Vec::new();
    intersections
        .try_reserve_exact(edges.len())
        .map_err(|_| PathError::AllocationFailed)?;
    let mut next_edge = 0;
    for y in min_y..max_y {
        let scan = y as f32 + 0.5;
        while next_edge < edges.len() && edges[next_edge].min_y <= scan {
            let edge = edges[next_edge];
            if edge.crosses(scan) {
                active_edges.push(edge);
            }
            next_edge += 1;
        }
        active_edges.retain(|edge| edge.crosses(scan));
        intersections.clear();
        intersections.extend(
            active_edges
                .iter()
                .map(|edge| (edge.x_at(scan), edge.winding)),
        );
        intersections.sort_unstable_by(|left, right| left.0.total_cmp(&right.0));

        let Some(row) = target.row_mut(y) else {
            continue;
        };
        let mut winding = 0_i32;
        let mut previous_x = 0.0_f32;
        let mut index = 0;
        while index < intersections.len() {
            let x = intersections[index].0;
            if winding != 0 && previous_x < x {
                let left = ((previous_x - 0.5).ceil().max(clip.x as f32) as u32).min(clip_right);
                let right = ((x - 0.5).ceil().max(clip.x as f32) as u32).min(clip_right);
                if left < right {
                    row[left as usize..right as usize].fill(color);
                }
            }
            while index < intersections.len() && intersections[index].0 == x {
                winding += intersections[index].1;
                index += 1;
            }
            previous_x = x;
        }
    }
    Ok(())
}

/// Fills a transformed path with antialiased non-zero winding coverage.
pub fn fill_path_non_zero_aa(
    target: &mut Framebuffer,
    path: &Path,
    color: Color,
    tolerance: f32,
) -> Result<(), PathError> {
    fill_transformed_path_non_zero_aa_clipped(
        target,
        path,
        Transform2D::IDENTITY,
        color,
        tolerance,
        Rect::new(0, 0, target.width(), target.height()),
    )
}

/// Fills a path with antialiased non-zero winding coverage inside a clip rectangle.
pub fn fill_path_non_zero_aa_clipped(
    target: &mut Framebuffer,
    path: &Path,
    color: Color,
    tolerance: f32,
    clip: Rect,
) -> Result<(), PathError> {
    fill_transformed_path_non_zero_aa_clipped(
        target,
        path,
        Transform2D::IDENTITY,
        color,
        tolerance,
        clip,
    )
}

/// Flattens and transforms a path before antialiased non-zero winding filling.
pub fn fill_transformed_path_non_zero_aa(
    target: &mut Framebuffer,
    path: &Path,
    transform: Transform2D,
    color: Color,
    tolerance: f32,
) -> Result<(), PathError> {
    fill_transformed_path_non_zero_aa_clipped(
        target,
        path,
        transform,
        color,
        tolerance,
        Rect::new(0, 0, target.width(), target.height()),
    )
}

/// Antialiased non-zero winding fill restricted to a framebuffer-space viewport.
pub fn fill_transformed_path_non_zero_aa_clipped(
    target: &mut Framebuffer,
    path: &Path,
    transform: Transform2D,
    color: Color,
    tolerance: f32,
    clip: Rect,
) -> Result<(), PathError> {
    const OFFSETS: [f32; 4] = [0.125, 0.375, 0.625, 0.875];
    let flattened = path.flatten(tolerance)?.try_transformed(transform)?;
    if color.a == 0 || target.width() == 0 || target.height() == 0 {
        return Ok(());
    }
    let Some(clip) = clip.intersection(Rect::new(0, 0, target.width(), target.height())) else {
        return Ok(());
    };
    let clip_right = clip.x.saturating_add(clip.width);
    let clip_bottom = clip.y.saturating_add(clip.height);
    let contours = try_collect_contours(&flattened)?;
    let mut edges = try_collect_fill_edges(&contours)?;
    if edges.is_empty() {
        return Ok(());
    }

    let (path_min_x, path_min_y, path_max_x, path_max_y) = edges.iter().fold(
        (
            f64::INFINITY,
            f32::INFINITY,
            f64::NEG_INFINITY,
            f32::NEG_INFINITY,
        ),
        |(min_x, min_y, max_x, max_y), edge| {
            let end_x =
                edge.min_x + edge.inverse_slope * (f64::from(edge.max_y) - f64::from(edge.min_y));
            (
                min_x.min(edge.min_x.min(end_x)),
                min_y.min(edge.min_y),
                max_x.max(edge.min_x.max(end_x)),
                max_y.max(edge.max_y),
            )
        },
    );
    let min_x = path_min_x
        .floor()
        .max(f64::from(clip.x))
        .min(f64::from(clip_right)) as u32;
    let max_x = path_max_x
        .ceil()
        .max(f64::from(clip.x))
        .min(f64::from(clip_right)) as u32;
    let min_y = path_min_y
        .floor()
        .max(clip.y as f32)
        .min(clip_bottom as f32) as u32;
    let max_y = path_max_y.ceil().max(clip.y as f32).min(clip_bottom as f32) as u32;
    if min_x >= max_x || min_y >= max_y {
        return Ok(());
    }

    edges.sort_unstable_by(|left, right| left.min_y.total_cmp(&right.min_y));
    let mut active_edges = Vec::new();
    active_edges
        .try_reserve_exact(edges.len())
        .map_err(|_| PathError::AllocationFailed)?;
    let mut intersections: Vec<(f32, i32)> = Vec::new();
    intersections
        .try_reserve_exact(edges.len())
        .map_err(|_| PathError::AllocationFailed)?;
    let mut coverage = try_zeroed_coverage((max_x - min_x) as usize)?;
    let mut next_edge = 0_usize;

    for y in min_y..max_y {
        coverage.fill(0);
        for offset_y in OFFSETS {
            let sample_y = y as f32 + offset_y;
            while next_edge < edges.len() && edges[next_edge].min_y <= sample_y {
                let edge = edges[next_edge];
                if edge.crosses(sample_y) {
                    active_edges.push(edge);
                }
                next_edge += 1;
            }
            active_edges.retain(|edge| edge.crosses(sample_y));
            intersections.clear();
            intersections.extend(
                active_edges
                    .iter()
                    .map(|edge| (edge.x_at(sample_y), edge.winding)),
            );
            intersections.sort_unstable_by(|left, right| left.0.total_cmp(&right.0));

            let mut winding = 0_i32;
            let mut previous_x = 0.0_f32;
            let mut index = 0_usize;
            while index < intersections.len() {
                let x = intersections[index].0;
                if winding != 0 && previous_x < x {
                    accumulate_horizontal_coverage(&mut coverage, min_x, max_x, previous_x, x);
                }
                while index < intersections.len() && intersections[index].0 == x {
                    winding += intersections[index].1;
                    index += 1;
                }
                previous_x = x;
            }
        }

        let Some(row) = target.row_mut(y) else {
            continue;
        };
        for (offset, &covered) in coverage.iter().enumerate() {
            if covered == 0 {
                continue;
            }
            let alpha = (u16::from(color.a) * u16::from(covered) / 16) as u8;
            if alpha == 0 {
                continue;
            }
            let source = Color::rgba(color.r, color.g, color.b, alpha);
            let index = min_x as usize + offset;
            row[index] = if alpha == 255 {
                source
            } else {
                source.blend_over(row[index])
            };
        }
    }
    Ok(())
}

/// Fills a path with 4x4 supersampled edge coverage and source-over blending.
pub fn fill_path_even_odd_aa(
    target: &mut Framebuffer,
    path: &Path,
    color: Color,
    tolerance: f32,
) -> Result<(), PathError> {
    fill_transformed_path_even_odd_aa(target, path, Transform2D::IDENTITY, color, tolerance)
}

/// Flattens, transforms and antialiases a path using the even-odd fill rule.
pub fn fill_transformed_path_even_odd_aa(
    target: &mut Framebuffer,
    path: &Path,
    transform: Transform2D,
    color: Color,
    tolerance: f32,
) -> Result<(), PathError> {
    let clip = Rect::new(0, 0, target.width(), target.height());
    fill_transformed_path_even_odd_aa_clipped(target, path, transform, color, tolerance, clip)
}

/// Antialiased path fill restricted to a framebuffer-space viewport.
pub fn fill_transformed_path_even_odd_aa_clipped(
    target: &mut Framebuffer,
    path: &Path,
    transform: Transform2D,
    color: Color,
    tolerance: f32,
    clip: Rect,
) -> Result<(), PathError> {
    let flattened = path.flatten(tolerance)?.try_transformed(transform)?;
    if color.a == 0 || target.width() == 0 || target.height() == 0 {
        return Ok(());
    }
    let Some(clip) = clip.intersection(Rect::new(0, 0, target.width(), target.height())) else {
        return Ok(());
    };
    let contours = try_collect_contours(&flattened)?;
    if contours.is_empty() {
        return Ok(());
    }

    let edges = try_collect_fill_edges(&contours)?;
    if edges.is_empty() {
        return Ok(());
    }

    const OFFSETS: [f32; 4] = [0.125, 0.375, 0.625, 0.875];
    let clip_right = clip.x.saturating_add(clip.width);
    let clip_bottom = clip.y.saturating_add(clip.height);
    let (path_min_x, path_min_y, path_max_x, path_max_y) =
        contours.iter().flat_map(|contour| contour.iter()).fold(
            (
                f32::INFINITY,
                f32::INFINITY,
                f32::NEG_INFINITY,
                f32::NEG_INFINITY,
            ),
            |(min_x, min_y, max_x, max_y), point| {
                (
                    min_x.min(point.x),
                    min_y.min(point.y),
                    max_x.max(point.x),
                    max_y.max(point.y),
                )
            },
        );
    let min_x = path_min_x.floor().max(clip.x as f32).min(clip_right as f32) as u32;
    let min_y = path_min_y
        .floor()
        .max(clip.y as f32)
        .min(clip_bottom as f32) as u32;
    let max_x = path_max_x.ceil().max(clip.x as f32).min(clip_right as f32) as u32;
    let max_y = path_max_y.ceil().min(clip_bottom as f32) as u32;
    if min_x >= max_x || min_y >= max_y {
        return Ok(());
    }

    // Coverage has only 17 possible values with 4x4 supersampling.
    // Precompute source-over results for opaque destination pixels so the
    // hot pixel loop performs table lookups instead of three channel divisions.
    let mut opaque_r = [[0_u8; 256]; 17];
    let mut opaque_g = [[0_u8; 256]; 17];
    let mut opaque_b = [[0_u8; 256]; 17];
    for covered in 1..=16 {
        let alpha = (u16::from(color.a) * covered as u16 / 16) as u32;
        let inverse_alpha = 255 - alpha;
        for destination in 0..256 {
            let destination = destination as u32;
            opaque_r[covered][destination as usize] =
                ((u32::from(color.r) * alpha + destination * inverse_alpha + 127) / 255) as u8;
            opaque_g[covered][destination as usize] =
                ((u32::from(color.g) * alpha + destination * inverse_alpha + 127) / 255) as u8;
            opaque_b[covered][destination as usize] =
                ((u32::from(color.b) * alpha + destination * inverse_alpha + 127) / 255) as u8;
        }
    }

    let width = (max_x - min_x) as usize;
    let mut coverage = try_zeroed_coverage(width)?;
    let mut intersections = Vec::new();
    intersections
        .try_reserve_exact(edges.len())
        .map_err(|_| PathError::AllocationFailed)?;
    // Samples advance monotonically from top to bottom, so retain only
    // edges that can still intersect the current subpixel scanline.
    let mut edges = edges;
    edges.sort_by(|left, right| left.min_y.total_cmp(&right.min_y));
    let mut active_edges = Vec::new();
    active_edges
        .try_reserve_exact(edges.len())
        .map_err(|_| PathError::AllocationFailed)?;
    let mut next_edge = 0;

    for y in min_y..max_y {
        coverage.fill(0);
        for offset_y in OFFSETS {
            let sample_y = y as f32 + offset_y;
            while next_edge < edges.len() && edges[next_edge].min_y <= sample_y {
                let edge = edges[next_edge];
                if edge.crosses(sample_y) {
                    active_edges.push(edge);
                }
                next_edge += 1;
            }
            active_edges.retain(|edge| edge.crosses(sample_y));
            intersections.clear();
            for &edge in &active_edges {
                intersections.push(edge.x_at(sample_y));
            }
            intersections.sort_by(f32::total_cmp);

            // Convert each even-odd span into horizontal subpixel coverage.
            // This avoids testing every edge for every individual sample.
            for pair in intersections.chunks_exact(2) {
                let left = pair[0];
                let right = pair[1];
                let span_start = ((left - 0.875).floor() as i64)
                    .max(i64::from(min_x))
                    .min(i64::from(max_x)) as u32;
                let span_end = ((right - 0.125).ceil() as i64)
                    .max(i64::from(min_x))
                    .min(i64::from(max_x)) as u32;
                for x in span_start..span_end {
                    let mut horizontal_samples = 0_u8;
                    for offset_x in OFFSETS {
                        let sample_x = x as f32 + offset_x;
                        if left <= sample_x && sample_x < right {
                            horizontal_samples += 1;
                        }
                    }
                    if horizontal_samples != 0 {
                        coverage[(x - min_x) as usize] += horizontal_samples;
                    }
                }
            }
        }

        let Some(row) = target.row_mut(y) else {
            continue;
        };
        for (offset, &covered) in coverage.iter().enumerate() {
            if covered == 0 {
                continue;
            }
            let alpha = (u16::from(color.a) * u16::from(covered) / 16) as u8;
            if alpha == 0 {
                continue;
            }
            let index = (min_x as usize) + offset;
            if alpha == u8::MAX {
                row[index] = Color::rgba(color.r, color.g, color.b, alpha);
            } else if row[index].a == u8::MAX {
                let coverage_index = usize::from(covered);
                row[index] = Color::rgb(
                    opaque_r[coverage_index][row[index].r as usize],
                    opaque_g[coverage_index][row[index].g as usize],
                    opaque_b[coverage_index][row[index].b as usize],
                );
            } else {
                let source = Color::rgba(color.r, color.g, color.b, alpha);
                row[index] = source.blend_over(row[index]);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        fill_path_even_odd, fill_path_even_odd_aa, fill_transformed_path_even_odd, Path, PathError,
    };
    use crate::{Color, Framebuffer, Rect, Transform2D, Vector};

    #[test]
    fn optimized_horizontal_coverage_matches_sample_by_sample_reference() {
        const OFFSETS: [f32; 4] = [0.125, 0.375, 0.625, 0.875];

        for left_step in -32..64 {
            for right_step in (left_step + 1)..96 {
                let left = left_step as f32 / 16.0;
                let right = right_step as f32 / 16.0;
                let mut actual = [0_u8; 8];
                let mut expected = [0_u8; 8];

                super::accumulate_horizontal_coverage(&mut actual, 0, 8, left, right);

                for pixel_x in 0..8 {
                    for offset_x in OFFSETS {
                        let sample_x = pixel_x as f32 + offset_x;
                        if left <= sample_x && sample_x < right {
                            expected[pixel_x as usize] += 1;
                        }
                    }
                }

                assert_eq!(actual, expected, "interval [{left}, {right})");
            }
        }
    }

    #[test]
    fn prepared_fill_edges_use_half_open_scanline_intervals() {
        let forward = super::FillEdge::new(Vector::new(2.0, 1.0), Vector::new(6.0, 5.0))
            .expect("non-horizontal edge");
        let reverse = super::FillEdge::new(Vector::new(6.0, 5.0), Vector::new(2.0, 1.0))
            .expect("reversed non-horizontal edge");

        for edge in [forward, reverse] {
            assert!(!edge.crosses(0.999));
            assert!(edge.crosses(1.0));
            assert!(edge.crosses(3.0));
            assert!(!edge.crosses(5.0));
            assert!((edge.x_at(3.0) - 4.0).abs() < f32::EPSILON);
        }

        assert!(super::FillEdge::new(Vector::new(1.0, 2.0), Vector::new(5.0, 2.0)).is_none());
    }

    #[test]
    fn fill_edge_interpolation_avoids_overflow_for_extreme_finite_coordinates() {
        let extent = f32::MAX;
        let edge = super::FillEdge::new(Vector::new(-extent, 0.0), Vector::new(extent, 2.0))
            .expect("non-horizontal edge");

        assert_eq!(edge.x_at(1.0), 0.0);
        assert!(edge.x_at(0.5).is_finite());
    }

    #[test]
    fn clipped_fill_handles_contours_spanning_extreme_finite_x_coordinates() {
        let extent = f32::MAX;
        let mut path = Path::new();
        path.move_to(Vector::new(-extent, 0.0))
            .line_to(Vector::new(extent, 0.0))
            .line_to(Vector::new(extent, 2.0))
            .line_to(Vector::new(-extent, 2.0))
            .close();

        let mut target = Framebuffer::new(4, 2, Color::BLACK).unwrap();
        fill_path_even_odd(&mut target, &path, Color::WHITE, 0.1).unwrap();

        assert!(target.pixels().iter().all(|pixel| *pixel == Color::WHITE));
    }

    #[test]
    fn all_fill_rules_handle_extreme_finite_coordinates() {
        let extent = f32::MAX;
        let mut path = Path::new();
        path.move_to(Vector::new(-extent, 0.0))
            .line_to(Vector::new(extent, 0.0))
            .line_to(Vector::new(extent, 2.0))
            .line_to(Vector::new(-extent, 2.0))
            .close();

        type DrawCase = fn(&mut Framebuffer, &Path, Color, f32) -> Result<(), PathError>;
        let draw_cases: [DrawCase; 4] = [
            fill_path_even_odd,
            super::fill_path_non_zero,
            fill_path_even_odd_aa,
            super::fill_path_non_zero_aa,
        ];

        for draw in draw_cases {
            let mut target = Framebuffer::new(4, 2, Color::BLACK).unwrap();
            draw(&mut target, &path, Color::WHITE, 0.1).unwrap();
            assert!(
                target.pixels().iter().all(|pixel| *pixel == Color::WHITE),
                "fill mode failed for a contour spanning finite f32 extremes"
            );
        }
    }

    #[test]
    fn active_edge_fill_matches_full_scan_for_complex_clipped_contours() {
        fn reference_full_scan(target: &mut Framebuffer, path: &Path, color: Color, clip: Rect) {
            let flattened = path.flatten(0.2).unwrap();
            let clip = clip
                .intersection(Rect::new(0, 0, target.width(), target.height()))
                .unwrap();
            let clip_right = clip.x.saturating_add(clip.width);
            let clip_bottom = clip.y.saturating_add(clip.height);
            let edges = flattened
                .subpaths
                .iter()
                .filter(|subpath| subpath.closed && subpath.points.len() >= 3)
                .flat_map(|subpath| {
                    (0..subpath.points.len()).filter_map(move |index| {
                        super::FillEdge::new(
                            subpath.points[index],
                            subpath.points[(index + 1) % subpath.points.len()],
                        )
                    })
                })
                .collect::<Vec<_>>();

            for y in clip.y..clip_bottom {
                let scan = y as f32 + 0.5;
                let mut intersections = edges
                    .iter()
                    .filter(|edge| edge.crosses(scan))
                    .map(|edge| edge.x_at(scan))
                    .collect::<Vec<_>>();
                intersections.sort_by(f32::total_cmp);

                let row = target.row_mut(y).unwrap();
                for pair in intersections.chunks_exact(2) {
                    let left = ((pair[0] - 0.5).ceil().max(clip.x as f32) as u32).min(clip_right);
                    let right = ((pair[1] - 0.5).ceil().max(clip.x as f32) as u32).min(clip_right);
                    if left < right {
                        row[left as usize..right as usize].fill(color);
                    }
                }
            }
        }

        let mut path = Path::new();
        // A concave contour partially outside the framebuffer.
        path.move_to(Vector::new(-3.0, 1.0))
            .line_to(Vector::new(13.0, 1.0))
            .line_to(Vector::new(13.0, 4.0))
            .line_to(Vector::new(7.0, 4.0))
            .line_to(Vector::new(7.0, 10.0))
            .line_to(Vector::new(-3.0, 10.0))
            .close();
        // A nested contour creates a hole.
        path.move_to(Vector::new(1.0, 3.0))
            .line_to(Vector::new(5.0, 3.0))
            .line_to(Vector::new(5.0, 8.0))
            .line_to(Vector::new(1.0, 8.0))
            .close();
        // A separate sloped contour exercises edges entering/leaving scanlines.
        path.move_to(Vector::new(10.0, 6.0))
            .line_to(Vector::new(16.0, 8.0))
            .line_to(Vector::new(12.0, 13.0))
            .close();

        let clip = Rect::new(2, 2, 10, 9);
        let mut optimized = Framebuffer::new(14, 12, Color::BLACK).unwrap();
        let mut reference = Framebuffer::new(14, 12, Color::BLACK).unwrap();

        super::fill_path_even_odd_clipped(
            &mut optimized,
            &path,
            Color::rgba(30, 120, 220, 255),
            0.2,
            clip,
        )
        .unwrap();
        reference_full_scan(&mut reference, &path, Color::rgba(30, 120, 220, 255), clip);

        assert_eq!(optimized.pixels(), reference.pixels());
    }

    #[test]
    fn quadratic_and_cubic_curves_flatten_to_multiple_segments() {
        let mut p = Path::new();
        p.move_to(Vector::new(0.0, 0.0))
            .quadratic_to(Vector::new(5.0, 10.0), Vector::new(10.0, 0.0))
            .cubic_to(
                Vector::new(12.0, -5.0),
                Vector::new(18.0, 5.0),
                Vector::new(20.0, 0.0),
            );
        let flat = p.flatten(0.1).unwrap();
        assert!(flat.subpaths[0].points.len() > 4);
        assert_eq!(
            *flat.subpaths[0].points.last().unwrap(),
            Vector::new(20.0, 0.0)
        );
    }

    #[test]
    fn fallible_flatten_handles_many_mixed_contours_and_preserves_path_semantics() {
        let mut path = Path::new();
        for index in 0..64 {
            let x = index as f32 * 12.0;
            let start = Vector::new(x, 1.0);
            path.move_to(start)
                .line_to(Vector::new(x + 4.0, 1.0))
                .quadratic_to(Vector::new(x + 6.0, 8.0), Vector::new(x + 8.0, 1.0))
                .cubic_to(
                    Vector::new(x + 9.0, -4.0),
                    Vector::new(x + 11.0, 6.0),
                    Vector::new(x + 12.0, 1.0),
                )
                .close()
                .line_to(Vector::new(x + 13.0, 2.0));
        }

        let flattened = path.try_flatten(0.05).unwrap();
        assert_eq!(flattened.subpaths.len(), 128);

        for pair in flattened.subpaths.chunks_exact(2) {
            assert!(pair[0].closed);
            assert!(!pair[1].closed);
            assert_eq!(pair[1].points.len(), 2);
            assert_eq!(pair[1].points[0], pair[0].points[0]);
            assert!(pair
                .iter()
                .flat_map(|subpath| &subpath.points)
                .all(|point| point.x.is_finite() && point.y.is_finite()));
        }
    }

    #[test]
    fn fallible_flatten_reports_invalid_input_without_partial_geometry() {
        let mut missing_start = Path::new();
        missing_start.try_line_to(Vector::new(1.0, 1.0)).unwrap();
        assert_eq!(
            missing_start.try_flatten(0.1),
            Err(PathError::MissingCurrentPoint)
        );

        let mut non_finite = Path::new();
        non_finite.try_move_to(Vector::new(f32::NAN, 0.0)).unwrap();
        assert_eq!(
            non_finite.try_flatten(0.1),
            Err(PathError::NonFiniteCoordinate)
        );

        let valid = Path::new();
        assert_eq!(
            valid.try_flatten(f32::INFINITY),
            Err(PathError::InvalidTolerance)
        );
        assert_eq!(valid.try_flatten(0.0), Err(PathError::InvalidTolerance));
    }

    #[test]
    fn fallible_flatten_bounds_subdivision_for_extreme_curve_tolerances() {
        let mut path = Path::new();
        path.move_to(Vector::new(0.0, 0.0))
            .quadratic_to(Vector::new(0.0, 1.0e6), Vector::new(1.0, 0.0))
            .cubic_to(
                Vector::new(1.0e6, -1.0e6),
                Vector::new(-1.0e6, 1.0e6),
                Vector::new(2.0, 0.0),
            );

        let flattened = path.try_flatten(1.0e-6).unwrap();
        let points = &flattened.subpaths[0].points;
        assert!(points.len() > 4);
        assert!(points.len() <= 2 * (1 << 16) + 1);
        assert_eq!(points.last(), Some(&Vector::new(2.0, 0.0)));
        assert!(points
            .iter()
            .all(|point| point.x.is_finite() && point.y.is_finite()));
    }

    #[test]
    fn collinear_quadratic_control_point_beyond_chord_is_not_flattened_away() {
        let mut path = Path::new();
        path.move_to(Vector::new(0.0, 0.0))
            .quadratic_to(Vector::new(20.0, 0.0), Vector::new(10.0, 0.0));

        let flattened = path.flatten(0.25).unwrap();
        let points = &flattened.subpaths[0].points;
        assert!(points.len() > 2);
        assert!(points.iter().any(|point| point.x > 10.0));
    }

    #[test]
    fn collinear_cubic_control_points_beyond_chord_preserve_overshoot() {
        let mut path = Path::new();
        path.move_to(Vector::new(0.0, 0.0)).cubic_to(
            Vector::new(20.0, 0.0),
            Vector::new(20.0, 0.0),
            Vector::new(10.0, 0.0),
        );

        let flattened = path.flatten(0.25).unwrap();
        let points = &flattened.subpaths[0].points;
        assert!(points.len() > 2);
        assert!(points.iter().any(|point| point.x > 10.0));
    }

    #[test]
    fn even_odd_fill_preserves_contours_thinner_than_f32_epsilon() {
        let top = 0.5 - f32::EPSILON / 4.0;
        let bottom = 0.5 + f32::EPSILON / 2.0;
        assert!((bottom - top).abs() < f32::EPSILON);

        let mut path = Path::new();
        path.move_to(Vector::new(0.0, top))
            .line_to(Vector::new(4.0, top))
            .line_to(Vector::new(4.0, bottom))
            .line_to(Vector::new(0.0, bottom))
            .close();

        let mut target = Framebuffer::new(4, 1, Color::BLACK).unwrap();
        fill_path_even_odd(&mut target, &path, Color::WHITE, 0.1).unwrap();

        assert!(target.pixels().iter().all(|pixel| *pixel == Color::WHITE));
    }

    #[test]
    fn drawing_after_close_starts_at_the_closed_contours_initial_point() {
        let start = Vector::new(1.0, 2.0);
        let mut path = Path::new();
        path.move_to(start)
            .line_to(Vector::new(5.0, 2.0))
            .line_to(Vector::new(5.0, 6.0))
            .close()
            .line_to(Vector::new(8.0, 2.0))
            .line_to(Vector::new(9.0, 4.0));

        let flattened = path.flatten(0.25).unwrap();
        assert_eq!(flattened.subpaths.len(), 2);
        assert!(flattened.subpaths[0].closed);
        assert_eq!(
            flattened.subpaths[1].points,
            [start, Vector::new(8.0, 2.0), Vector::new(9.0, 4.0)]
        );
        assert!(!flattened.subpaths[1].closed);
    }

    #[test]
    fn curve_after_close_starts_a_new_contour_at_the_closed_start() {
        let start = Vector::new(2.0, 3.0);
        let mut path = Path::new();
        path.move_to(start)
            .line_to(Vector::new(6.0, 3.0))
            .line_to(Vector::new(6.0, 7.0))
            .close()
            .quadratic_to(Vector::new(4.0, 10.0), Vector::new(8.0, 9.0));

        let flattened = path.flatten(0.1).unwrap();
        assert_eq!(flattened.subpaths.len(), 2);
        assert_eq!(flattened.subpaths[1].points.first(), Some(&start));
        assert_eq!(
            flattened.subpaths[1].points.last(),
            Some(&Vector::new(8.0, 9.0))
        );
    }

    #[test]
    fn closed_path_fills_and_open_path_does_not() {
        let mut p = Path::new();
        p.move_to(Vector::new(1.0, 1.0))
            .line_to(Vector::new(5.0, 1.0))
            .line_to(Vector::new(5.0, 5.0))
            .line_to(Vector::new(1.0, 5.0))
            .close();
        let mut fb = Framebuffer::new(7, 7, Color::BLACK).unwrap();
        fill_path_even_odd(&mut fb, &p, Color::RED, 0.25).unwrap();
        assert_eq!(fb.get_pixel(2, 2), Some(Color::RED));
        assert_eq!(fb.get_pixel(0, 0), Some(Color::BLACK));
    }

    #[test]
    fn axis_aligned_rectangle_matches_exact_pixel_reference() {
        let mut path = Path::new();
        path.move_to(Vector::new(1.0, 1.0))
            .line_to(Vector::new(5.0, 1.0))
            .line_to(Vector::new(5.0, 5.0))
            .line_to(Vector::new(1.0, 5.0))
            .close();

        let mut framebuffer = Framebuffer::new(7, 7, Color::BLACK).unwrap();
        fill_path_even_odd(&mut framebuffer, &path, Color::RED, 0.25).unwrap();

        for y in 0..7 {
            for x in 0..7 {
                let expected = if (1..5).contains(&x) && (1..5).contains(&y) {
                    Color::RED
                } else {
                    Color::BLACK
                };
                assert_eq!(
                    framebuffer.get_pixel(x, y),
                    Some(expected),
                    "unexpected pixel at ({x}, {y})"
                );
            }
        }
    }

    #[test]
    fn clipped_path_fill_respects_viewport_and_framebuffer() {
        let mut path = Path::new();
        path.move_to(Vector::new(1.0, 1.0))
            .line_to(Vector::new(7.0, 1.0))
            .line_to(Vector::new(7.0, 7.0))
            .line_to(Vector::new(1.0, 7.0))
            .close();
        let mut fb = Framebuffer::new(9, 9, Color::BLACK).unwrap();
        super::fill_path_even_odd_clipped(&mut fb, &path, Color::BLUE, 0.25, Rect::new(3, 2, 2, 3))
            .unwrap();
        assert_eq!(fb.get_pixel(3, 2), Some(Color::BLUE));
        assert_eq!(fb.get_pixel(4, 4), Some(Color::BLUE));
        assert_eq!(fb.get_pixel(2, 3), Some(Color::BLACK));
        assert_eq!(fb.get_pixel(5, 3), Some(Color::BLACK));
        assert_eq!(fb.get_pixel(3, 5), Some(Color::BLACK));
    }

    #[test]
    fn clipped_path_fill_with_empty_viewport_is_a_noop() {
        let mut path = Path::new();
        path.move_to(Vector::new(0.0, 0.0))
            .line_to(Vector::new(4.0, 0.0))
            .line_to(Vector::new(4.0, 4.0))
            .close();
        let mut fb = Framebuffer::new(5, 5, Color::BLACK).unwrap();
        super::fill_path_even_odd_clipped(
            &mut fb,
            &path,
            Color::RED,
            0.25,
            Rect::new(10, 10, 2, 2),
        )
        .unwrap();
        assert!(fb.pixels().iter().all(|pixel| *pixel == Color::BLACK));
    }

    #[test]
    fn nested_contours_create_even_odd_hole() {
        let mut p = Path::new();
        p.move_to(Vector::new(0.0, 0.0))
            .line_to(Vector::new(8.0, 0.0))
            .line_to(Vector::new(8.0, 8.0))
            .line_to(Vector::new(0.0, 8.0))
            .close();
        p.move_to(Vector::new(2.0, 2.0))
            .line_to(Vector::new(6.0, 2.0))
            .line_to(Vector::new(6.0, 6.0))
            .line_to(Vector::new(2.0, 6.0))
            .close();
        let mut fb = Framebuffer::new(8, 8, Color::BLACK).unwrap();
        fill_path_even_odd(&mut fb, &p, Color::GREEN, 0.25).unwrap();
        assert_eq!(fb.get_pixel(1, 1), Some(Color::GREEN));
        assert_eq!(fb.get_pixel(3, 3), Some(Color::BLACK));
    }

    #[test]
    fn non_zero_fill_keeps_same_direction_nested_contours_filled() {
        let mut path = Path::new();
        path.move_to(Vector::new(0.0, 0.0))
            .line_to(Vector::new(8.0, 0.0))
            .line_to(Vector::new(8.0, 8.0))
            .line_to(Vector::new(0.0, 8.0))
            .close();
        // Same winding direction: non-zero fills the nested contour too.
        path.move_to(Vector::new(2.0, 2.0))
            .line_to(Vector::new(6.0, 2.0))
            .line_to(Vector::new(6.0, 6.0))
            .line_to(Vector::new(2.0, 6.0))
            .close();
        let mut fb = Framebuffer::new(8, 8, Color::BLACK).unwrap();
        super::fill_path_non_zero(&mut fb, &path, Color::GREEN, 0.2).unwrap();
        assert_eq!(fb.get_pixel(1, 1), Some(Color::GREEN));
        assert_eq!(fb.get_pixel(3, 3), Some(Color::GREEN));
    }

    #[test]
    fn non_zero_fill_creates_hole_for_opposite_winding_contour() {
        let mut path = Path::new();
        path.move_to(Vector::new(0.0, 0.0))
            .line_to(Vector::new(8.0, 0.0))
            .line_to(Vector::new(8.0, 8.0))
            .line_to(Vector::new(0.0, 8.0))
            .close();
        // Reverse direction relative to the outer contour.
        path.move_to(Vector::new(2.0, 2.0))
            .line_to(Vector::new(2.0, 6.0))
            .line_to(Vector::new(6.0, 6.0))
            .line_to(Vector::new(6.0, 2.0))
            .close();
        let mut fb = Framebuffer::new(8, 8, Color::BLACK).unwrap();
        super::fill_path_non_zero_clipped(&mut fb, &path, Color::GREEN, 0.2, Rect::new(1, 1, 6, 6))
            .unwrap();
        assert_eq!(fb.get_pixel(1, 1), Some(Color::GREEN));
        assert_eq!(fb.get_pixel(3, 3), Some(Color::BLACK));
    }

    #[test]
    fn transformed_path_is_rasterized_at_transformed_coordinates() {
        let mut path = Path::new();
        path.move_to(Vector::new(0.0, 0.0))
            .line_to(Vector::new(3.0, 0.0))
            .line_to(Vector::new(3.0, 3.0))
            .line_to(Vector::new(0.0, 3.0))
            .close();
        let mut fb = Framebuffer::new(12, 10, Color::BLACK).unwrap();
        let transform = Transform2D::scale(2.0, 1.0).then(Transform2D::translation(5.0, 4.0));
        fill_transformed_path_even_odd(&mut fb, &path, transform, Color::BLUE, 0.2).unwrap();
        assert_eq!(fb.get_pixel(6, 5), Some(Color::BLUE));
        assert_eq!(fb.get_pixel(1, 1), Some(Color::BLACK));
    }

    #[test]
    fn transformed_path_rejects_non_finite_output() {
        let mut path = Path::new();
        path.move_to(Vector::new(1.0, 1.0))
            .line_to(Vector::new(2.0, 1.0))
            .line_to(Vector::new(2.0, 2.0))
            .close();
        let mut fb = Framebuffer::new(4, 4, Color::BLACK).unwrap();
        let invalid = Transform2D::new(f32::INFINITY, 0.0, 0.0, 1.0, 0.0, 0.0);
        assert_eq!(
            fill_transformed_path_even_odd(&mut fb, &path, invalid, Color::RED, 0.2),
            Err(PathError::InvalidTransform)
        );
    }

    #[test]
    fn invalid_path_operations_and_coordinates_are_reported() {
        let mut p = Path::new();
        p.line_to(Vector::new(1.0, 1.0));
        assert_eq!(p.flatten(0.1), Err(PathError::MissingCurrentPoint));
        let mut p = Path::new();
        p.move_to(Vector::new(f32::NAN, 0.0));
        assert_eq!(p.flatten(0.1), Err(PathError::NonFiniteCoordinate));
        assert_eq!(Path::new().flatten(0.0), Err(PathError::InvalidTolerance));
    }

    #[test]
    fn fallible_path_builders_match_the_convenience_builders() {
        let start = Vector::new(1.0, 2.0);
        let mut fallible = Path::new();
        fallible.try_reserve_commands(8).unwrap();
        fallible
            .try_move_to(start)
            .unwrap()
            .try_line_to(Vector::new(5.0, 2.0))
            .unwrap()
            .try_quadratic_to(Vector::new(6.0, 4.0), Vector::new(5.0, 6.0))
            .unwrap()
            .try_cubic_to(
                Vector::new(4.0, 7.0),
                Vector::new(2.0, 7.0),
                Vector::new(1.0, 6.0),
            )
            .unwrap()
            .try_close()
            .unwrap();

        let mut convenience = Path::new();
        convenience
            .move_to(start)
            .line_to(Vector::new(5.0, 2.0))
            .quadratic_to(Vector::new(6.0, 4.0), Vector::new(5.0, 6.0))
            .cubic_to(
                Vector::new(4.0, 7.0),
                Vector::new(2.0, 7.0),
                Vector::new(1.0, 6.0),
            )
            .close();

        assert_eq!(fallible.flatten(0.1), convenience.flatten(0.1));
    }

    #[test]
    fn fallible_transformed_path_matches_convenience_transform_and_preserves_source() {
        let mut path = Path::new();
        path.move_to(Vector::new(0.0, 0.0))
            .line_to(Vector::new(3.0, 0.0))
            .line_to(Vector::new(3.0, 2.0))
            .close();
        let flattened = path.flatten(0.2).unwrap();
        let original = flattened.clone();
        let transform = Transform2D::scale(2.0, 3.0).then(Transform2D::translation(4.0, 5.0));

        let fallible = flattened.try_transformed(transform).unwrap();
        let convenience = flattened.transformed(transform).unwrap();

        assert_eq!(fallible, convenience);
        assert_eq!(flattened, original);
        assert_eq!(fallible.subpaths[0].points[0], Vector::new(4.0, 5.0));
        assert!(fallible.subpaths[0].closed);
    }

    #[test]
    fn fallible_transform_rejects_non_finite_output_without_mutating_source() {
        let mut path = Path::new();
        path.move_to(Vector::new(1.0, 1.0))
            .line_to(Vector::new(2.0, 1.0))
            .close();
        let flattened = path.flatten(0.2).unwrap();
        let original = flattened.clone();
        let invalid = Transform2D::new(f32::INFINITY, 0.0, 0.0, 1.0, 0.0, 0.0);

        assert_eq!(
            flattened.try_transformed(invalid),
            Err(PathError::InvalidTransform)
        );
        assert_eq!(flattened, original);
    }

    #[test]
    fn antialiased_active_edge_fill_matches_full_scan_reference() {
        fn reference_full_scan(target: &mut Framebuffer, path: &Path, color: Color, clip: Rect) {
            const OFFSETS: [f32; 4] = [0.125, 0.375, 0.625, 0.875];
            let flattened = path.flatten(0.2).unwrap();
            let clip = clip
                .intersection(Rect::new(0, 0, target.width(), target.height()))
                .unwrap();
            let clip_right = clip.x.saturating_add(clip.width);
            let clip_bottom = clip.y.saturating_add(clip.height);
            let contours: Vec<&[Vector]> = flattened
                .subpaths
                .iter()
                .filter(|subpath| subpath.closed && subpath.points.len() >= 3)
                .map(|subpath| subpath.points.as_slice())
                .collect();
            let edges = contours
                .iter()
                .flat_map(|contour| {
                    (0..contour.len()).filter_map(move |index| {
                        super::FillEdge::new(contour[index], contour[(index + 1) % contour.len()])
                    })
                })
                .collect::<Vec<_>>();
            let min_x = contours
                .iter()
                .flat_map(|contour| contour.iter().map(|point| point.x))
                .fold(f32::INFINITY, f32::min)
                .floor()
                .max(clip.x as f32)
                .min(clip_right as f32) as u32;
            let max_x = contours
                .iter()
                .flat_map(|contour| contour.iter().map(|point| point.x))
                .fold(f32::NEG_INFINITY, f32::max)
                .ceil()
                .max(clip.x as f32)
                .min(clip_right as f32) as u32;
            let min_y = contours
                .iter()
                .flat_map(|contour| contour.iter().map(|point| point.y))
                .fold(f32::INFINITY, f32::min)
                .floor()
                .max(clip.y as f32)
                .min(clip_bottom as f32) as u32;
            let max_y = contours
                .iter()
                .flat_map(|contour| contour.iter().map(|point| point.y))
                .fold(f32::NEG_INFINITY, f32::max)
                .ceil()
                .min(clip_bottom as f32) as u32;

            for y in min_y..max_y {
                let mut row_coverage = vec![0_u8; (max_x - min_x) as usize];
                for offset_y in OFFSETS {
                    let sample_y = y as f32 + offset_y;
                    let mut intersections = edges
                        .iter()
                        .filter(|edge| edge.crosses(sample_y))
                        .map(|edge| edge.x_at(sample_y))
                        .collect::<Vec<_>>();
                    intersections.sort_by(f32::total_cmp);
                    for pair in intersections.chunks_exact(2) {
                        for x in min_x..max_x {
                            let mut count = 0_u8;
                            for offset_x in OFFSETS {
                                let sample_x = x as f32 + offset_x;
                                if pair[0] <= sample_x && sample_x < pair[1] {
                                    count += 1;
                                }
                            }
                            row_coverage[(x - min_x) as usize] += count;
                        }
                    }
                }

                let row = target.row_mut(y).unwrap();
                for (offset, &covered) in row_coverage.iter().enumerate() {
                    if covered == 0 {
                        continue;
                    }
                    let alpha = (u16::from(color.a) * u16::from(covered) / 16) as u8;
                    if alpha == 0 {
                        continue;
                    }
                    let source = Color::rgba(color.r, color.g, color.b, alpha);
                    let index = min_x as usize + offset;
                    row[index] = if alpha == u8::MAX {
                        source
                    } else {
                        source.blend_over(row[index])
                    };
                }
            }
        }

        let mut path = Path::new();
        path.move_to(Vector::new(-2.4, 0.6))
            .line_to(Vector::new(13.2, 1.1))
            .line_to(Vector::new(12.4, 5.0))
            .line_to(Vector::new(7.1, 4.6))
            .line_to(Vector::new(6.7, 10.8))
            .line_to(Vector::new(-1.8, 9.7))
            .close();
        path.move_to(Vector::new(1.2, 2.8))
            .line_to(Vector::new(5.1, 2.5))
            .line_to(Vector::new(5.4, 7.9))
            .line_to(Vector::new(1.0, 8.1))
            .close();
        path.move_to(Vector::new(9.8, 6.2))
            .line_to(Vector::new(16.1, 8.4))
            .line_to(Vector::new(11.8, 13.1))
            .close();

        let clip = Rect::new(1, 1, 11, 10);
        let background = Color::rgba(18, 27, 39, 255);
        let color = Color::rgba(40, 190, 230, 173);
        let mut optimized = Framebuffer::new(14, 12, background).unwrap();
        let mut reference = Framebuffer::new(14, 12, background).unwrap();

        super::fill_transformed_path_even_odd_aa_clipped(
            &mut optimized,
            &path,
            Transform2D::IDENTITY,
            color,
            0.2,
            clip,
        )
        .unwrap();
        reference_full_scan(&mut reference, &path, color, clip);

        assert_eq!(optimized.pixels(), reference.pixels());
    }

    #[test]
    fn non_zero_antialiased_fill_preserves_same_direction_nested_contours() {
        let mut path = Path::new();
        path.move_to(Vector::new(0.25, 0.25))
            .line_to(Vector::new(8.25, 0.25))
            .line_to(Vector::new(8.25, 8.25))
            .line_to(Vector::new(0.25, 8.25))
            .close();
        path.move_to(Vector::new(2.0, 2.0))
            .line_to(Vector::new(6.0, 2.0))
            .line_to(Vector::new(6.0, 6.0))
            .line_to(Vector::new(2.0, 6.0))
            .close();

        let mut target = Framebuffer::new(10, 10, Color::BLACK).unwrap();
        super::fill_path_non_zero_aa(&mut target, &path, Color::WHITE, 0.2).unwrap();
        assert_eq!(target.get_pixel(4, 4), Some(Color::WHITE));
        assert!(target
            .pixels()
            .iter()
            .any(|pixel| pixel.r > 0 && pixel.r < 255));
    }

    #[test]
    fn non_zero_antialiased_fill_respects_opposite_winding_holes() {
        let mut path = Path::new();
        path.move_to(Vector::new(0.0, 0.0))
            .line_to(Vector::new(8.0, 0.0))
            .line_to(Vector::new(8.0, 8.0))
            .line_to(Vector::new(0.0, 8.0))
            .close();
        path.move_to(Vector::new(2.0, 2.0))
            .line_to(Vector::new(2.0, 6.0))
            .line_to(Vector::new(6.0, 6.0))
            .line_to(Vector::new(6.0, 2.0))
            .close();
        let mut target = Framebuffer::new(8, 8, Color::BLACK).unwrap();
        super::fill_path_non_zero_aa_clipped(
            &mut target,
            &path,
            Color::WHITE,
            0.2,
            Rect::new(1, 1, 6, 6),
        )
        .unwrap();
        assert_eq!(target.get_pixel(3, 3), Some(Color::BLACK));
        assert_eq!(target.get_pixel(1, 1), Some(Color::WHITE));
    }

    #[test]
    fn antialiased_path_blends_partial_edge_coverage() {
        let mut path = Path::new();
        path.move_to(Vector::new(1.25, 1.25))
            .line_to(Vector::new(6.25, 1.25))
            .line_to(Vector::new(6.25, 6.25))
            .line_to(Vector::new(1.25, 6.25))
            .close();
        let mut fb = Framebuffer::new(8, 8, Color::BLACK).unwrap();
        fill_path_even_odd_aa(&mut fb, &path, Color::WHITE, 0.2).unwrap();
        assert!(fb.pixels().iter().any(|pixel| pixel.r > 0 && pixel.r < 255));
        assert!(fb.pixels().contains(&Color::WHITE));
    }

    #[test]
    fn antialiased_path_preserves_even_odd_holes_and_source_alpha() {
        let mut path = Path::new();
        path.move_to(Vector::new(0.0, 0.0))
            .line_to(Vector::new(8.0, 0.0))
            .line_to(Vector::new(8.0, 8.0))
            .line_to(Vector::new(0.0, 8.0))
            .close();
        path.move_to(Vector::new(2.0, 2.0))
            .line_to(Vector::new(6.0, 2.0))
            .line_to(Vector::new(6.0, 6.0))
            .line_to(Vector::new(2.0, 6.0))
            .close();
        let mut fb = Framebuffer::new(8, 8, Color::BLACK).unwrap();
        fill_path_even_odd_aa(&mut fb, &path, Color::rgba(0, 255, 0, 128), 0.2).unwrap();
        assert_eq!(fb.get_pixel(3, 3), Some(Color::BLACK));
        assert!(fb.get_pixel(1, 1).unwrap().g > 0);
        assert!(fb.get_pixel(1, 1).unwrap().g < 255);
    }

    #[test]
    fn antialiased_path_respects_clip_viewport() {
        let mut path = Path::new();
        path.move_to(Vector::new(0.0, 0.0))
            .line_to(Vector::new(8.0, 0.0))
            .line_to(Vector::new(8.0, 8.0))
            .line_to(Vector::new(0.0, 8.0))
            .close();
        let mut fb = Framebuffer::new(8, 8, Color::BLACK).unwrap();
        super::fill_transformed_path_even_odd_aa_clipped(
            &mut fb,
            &path,
            Transform2D::IDENTITY,
            Color::RED,
            0.2,
            Rect::new(2, 3, 2, 2),
        )
        .unwrap();
        for y in 0..8 {
            for x in 0..8 {
                let inside = (2..4).contains(&x) && (3..5).contains(&y);
                assert_eq!(fb.get_pixel(x, y) != Some(Color::BLACK), inside);
            }
        }
    }

    #[test]
    fn curve_flattening_handles_extreme_finite_coordinates_without_overflow() {
        let extent = f32::MAX;
        let mut path = Path::new();
        path.move_to(Vector::new(-extent, 0.0))
            .quadratic_to(Vector::new(0.0, extent), Vector::new(extent, 0.0));

        let flattened = path.flatten(1.0e30).unwrap();
        let points = &flattened.subpaths[0].points;
        assert!(points.len() > 2);
        assert!(points
            .iter()
            .all(|point| point.x.is_finite() && point.y.is_finite()));
        assert_eq!(points.first(), Some(&Vector::new(-extent, 0.0)));
        assert_eq!(points.last(), Some(&Vector::new(extent, 0.0)));
    }

    #[test]
    fn path_command_storage_can_be_reused_trimmed_and_released() {
        let mut path = Path::new();
        path.try_reserve_commands(16).unwrap();
        let capacity = path.command_capacity();
        path.move_to(Vector::new(1.0, 2.0))
            .line_to(Vector::new(3.0, 4.0))
            .close();
        let allocated = path.allocated_bytes();
        assert!(allocated >= 16 * core::mem::size_of::<super::Command>());
        assert!(capacity >= 16);
        path.clear();
        assert_eq!(path.command_capacity(), capacity);
        assert_eq!(path.allocated_bytes(), allocated);
        assert_eq!(path.trim_to_budget(allocated), 0);
        assert!(path.trim_to_budget(0) > 0);
        assert_eq!(path.command_capacity(), 0);
        assert_eq!(path.allocated_bytes(), 0);
        path.move_to(Vector::new(5.0, 6.0));
        assert_eq!(path.flatten(0.25).unwrap().subpaths.len(), 1);
        path.release();
        assert_eq!(path.allocated_bytes(), 0);
    }

}
