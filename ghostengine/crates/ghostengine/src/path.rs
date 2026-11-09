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
}

impl Path {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            commands: Vec::new(),
        }
    }

    pub fn move_to(&mut self, point: Vector) -> &mut Self {
        self.commands.push(Command::MoveTo(point));
        self
    }
    pub fn line_to(&mut self, point: Vector) -> &mut Self {
        self.commands.push(Command::LineTo(point));
        self
    }
    pub fn quadratic_to(&mut self, control: Vector, end: Vector) -> &mut Self {
        self.commands.push(Command::QuadTo(control, end));
        self
    }
    pub fn cubic_to(&mut self, c1: Vector, c2: Vector, end: Vector) -> &mut Self {
        self.commands.push(Command::CubicTo(c1, c2, end));
        self
    }
    pub fn close(&mut self) -> &mut Self {
        self.commands.push(Command::Close);
        self
    }

    /// Flattens curves until their control points are within tolerance of the chord.
    /// Tolerance is measured in the path's local coordinate space.
    pub fn flatten(&self, tolerance: f32) -> Result<FlattenedPath, PathError> {
        if !tolerance.is_finite() || tolerance <= 0.0 {
            return Err(PathError::InvalidTolerance);
        }
        let mut result = FlattenedPath::default();
        let mut current: Option<usize> = None;
        for command in &self.commands {
            match *command {
                Command::MoveTo(p) => {
                    validate(p)?;
                    result.subpaths.push(FlattenedSubpath {
                        points: vec![p],
                        closed: false,
                    });
                    current = Some(result.subpaths.len() - 1);
                }
                Command::LineTo(p) => {
                    validate(p)?;
                    let contour = current
                        .and_then(|i| result.subpaths.get_mut(i))
                        .ok_or(PathError::MissingCurrentPoint)?;
                    contour.points.push(p);
                }
                Command::QuadTo(control, end) => {
                    validate(control)?;
                    validate(end)?;
                    let contour = current
                        .and_then(|i| result.subpaths.get_mut(i))
                        .ok_or(PathError::MissingCurrentPoint)?;
                    let start = *contour
                        .points
                        .last()
                        .ok_or(PathError::MissingCurrentPoint)?;
                    flatten_quad(start, control, end, tolerance, 0, &mut contour.points);
                }
                Command::CubicTo(c1, c2, end) => {
                    validate(c1)?;
                    validate(c2)?;
                    validate(end)?;
                    let contour = current
                        .and_then(|i| result.subpaths.get_mut(i))
                        .ok_or(PathError::MissingCurrentPoint)?;
                    let start = *contour
                        .points
                        .last()
                        .ok_or(PathError::MissingCurrentPoint)?;
                    flatten_cubic(start, c1, c2, end, tolerance, 0, &mut contour.points);
                }
                Command::Close => {
                    let index = current.ok_or(PathError::MissingCurrentPoint)?;
                    result.subpaths[index].closed = true;
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
fn distance_to_segment(p: Vector, a: Vector, b: Vector) -> f32 {
    let dx = b.x - a.x;
    let dy = b.y - a.y;
    let length_squared = dx * dx + dy * dy;
    if length_squared <= f32::EPSILON {
        return (p.x - a.x).hypot(p.y - a.y);
    }

    let projection = (((p.x - a.x) * dx + (p.y - a.y) * dy) / length_squared).clamp(0.0, 1.0);
    let closest_x = a.x + projection * dx;
    let closest_y = a.y + projection * dy;
    (p.x - closest_x).hypot(p.y - closest_y)
}
fn midpoint(a: Vector, b: Vector) -> Vector {
    Vector::new((a.x + b.x) * 0.5, (a.y + b.y) * 0.5)
}

fn flatten_quad(a: Vector, c: Vector, b: Vector, tol: f32, depth: u8, out: &mut Vec<Vector>) {
    if depth >= 16 || distance_to_segment(c, a, b) <= tol {
        out.push(b);
        return;
    }
    let ac = midpoint(a, c);
    let cb = midpoint(c, b);
    let m = midpoint(ac, cb);
    flatten_quad(a, ac, m, tol, depth + 1, out);
    flatten_quad(m, cb, b, tol, depth + 1, out);
}
fn flatten_cubic(
    a: Vector,
    c1: Vector,
    c2: Vector,
    b: Vector,
    tol: f32,
    depth: u8,
    out: &mut Vec<Vector>,
) {
    if depth >= 16 || distance_to_segment(c1, a, b).max(distance_to_segment(c2, a, b)) <= tol {
        out.push(b);
        return;
    }
    let a1 = midpoint(a, c1);
    let c = midpoint(c1, c2);
    let b2 = midpoint(c2, b);
    let a2 = midpoint(a1, c);
    let b1 = midpoint(c, b2);
    let m = midpoint(a2, b1);
    flatten_cubic(a, a1, a2, m, tol, depth + 1, out);
    flatten_cubic(m, b1, b2, b, tol, depth + 1, out);
}

/// Transforms flattened geometry without changing the source path.
impl FlattenedPath {
    pub fn transformed(&self, transform: Transform2D) -> Result<Self, PathError> {
        if !transform.is_finite() {
            return Err(PathError::InvalidTransform);
        }
        let mut result = self.clone();
        for subpath in &mut result.subpaths {
            for point in &mut subpath.points {
                *point = transform.transform_point(*point);
                validate(*point)?;
            }
        }
        Ok(result)
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
    let flattened = path.flatten(tolerance)?.transformed(transform)?;
    let Some(clip) = clip.intersection(Rect::new(0, 0, target.width(), target.height())) else {
        return Ok(());
    };
    if target.width() == 0 || target.height() == 0 {
        return Ok(());
    }
    let clip_right = clip.x.saturating_add(clip.width);
    let clip_bottom = clip.y.saturating_add(clip.height);
    let contours: Vec<&[Vector]> = flattened
        .subpaths
        .iter()
        .filter(|s| s.closed && s.points.len() >= 3)
        .map(|s| s.points.as_slice())
        .collect();
    if contours.is_empty() {
        return Ok(());
    }

    let min_y = contours
        .iter()
        .flat_map(|c| c.iter().map(|p| p.y))
        .fold(f32::INFINITY, f32::min)
        .floor()
        .max(clip.y as f32)
        .min(clip_bottom as f32) as u32;
    let max_y = contours
        .iter()
        .flat_map(|c| c.iter().map(|p| p.y))
        .fold(f32::NEG_INFINITY, f32::max)
        .ceil()
        .max(clip.y as f32)
        .min(clip_bottom as f32) as u32;
    let mut intersections = Vec::new();
    for y in min_y..max_y {
        intersections.clear();
        let scan = y as f32 + 0.5;
        for contour in &contours {
            for i in 0..contour.len() {
                let a = contour[i];
                let b = contour[(i + 1) % contour.len()];
                if (a.y <= scan && scan < b.y) || (b.y <= scan && scan < a.y) {
                    let t = (scan - a.y) / (b.y - a.y);
                    intersections.push(a.x + t * (b.x - a.x));
                }
            }
        }
        intersections.sort_by(f32::total_cmp);
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

/// Fills a path with 4x4 supersampled edge coverage and source-over blending.
pub fn fill_path_even_odd_aa(
    target: &mut Framebuffer,
    path: &Path,
    color: Color,
    tolerance: f32,
) -> Result<(), PathError> {
    fill_transformed_path_even_odd_aa(
        target,
        path,
        Transform2D::IDENTITY,
        color,
        tolerance,
    )
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
    fill_transformed_path_even_odd_aa_clipped(
        target, path, transform, color, tolerance, clip,
    )
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
    let flattened = path.flatten(tolerance)?.transformed(transform)?;
    if color.a == 0 || target.width() == 0 || target.height() == 0 {
        return Ok(());
    }
    let Some(clip) = clip.intersection(Rect::new(0, 0, target.width(), target.height())) else {
        return Ok(());
    };
    let contours: Vec<&[Vector]> = flattened
        .subpaths
        .iter()
        .filter(|subpath| subpath.closed && subpath.points.len() >= 3)
        .map(|subpath| subpath.points.as_slice())
        .collect();
    if contours.is_empty() {
        return Ok(());
    }

    const OFFSETS: [f32; 4] = [0.125, 0.375, 0.625, 0.875];
    let min_x = contours
        .iter()
        .flat_map(|contour| contour.iter().map(|point| point.x))
        .fold(f32::INFINITY, f32::min)
        .floor()
        .max(clip.x as f32)
        .min(clip.x.saturating_add(clip.width) as f32) as u32;
    let min_y = contours
        .iter()
        .flat_map(|contour| contour.iter().map(|point| point.y))
        .fold(f32::INFINITY, f32::min)
        .floor()
        .max(clip.y as f32)
        .min(clip.y.saturating_add(clip.height) as f32) as u32;
    let max_x = contours
        .iter()
        .flat_map(|contour| contour.iter().map(|point| point.x))
        .fold(f32::NEG_INFINITY, f32::max)
        .ceil()
        .max(clip.x as f32)
        .min(clip.x.saturating_add(clip.width) as f32) as u32;
    let max_y = contours
        .iter()
        .flat_map(|contour| contour.iter().map(|point| point.y))
        .fold(f32::NEG_INFINITY, f32::max)
        .ceil()
        .max(clip.y as f32)
        .min(clip.y.saturating_add(clip.height) as f32) as u32;

    for y in min_y..max_y {
        let Some(row) = target.row_mut(y) else {
            continue;
        };
        for x in min_x..max_x {
            let mut covered = 0_u16;
            for offset_y in OFFSETS {
                for offset_x in OFFSETS {
                    let sample_x = x as f32 + offset_x;
                    let sample_y = y as f32 + offset_y;
                    let mut inside = false;
                    for contour in &contours {
                        for index in 0..contour.len() {
                            let a = contour[index];
                            let b = contour[(index + 1) % contour.len()];
                            if (a.y > sample_y) != (b.y > sample_y) {
                                let crossing_x =
                                    a.x + (sample_y - a.y) * (b.x - a.x) / (b.y - a.y);
                                if sample_x < crossing_x {
                                    inside = !inside;
                                }
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
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{fill_path_even_odd, fill_transformed_path_even_odd, Path, PathError};
    use crate::{Color, Framebuffer, Rect, Transform2D, Vector};

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
}
