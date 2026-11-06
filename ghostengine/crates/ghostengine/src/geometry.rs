/// A point in 2D integer pixel coordinates.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

impl Point {
    #[must_use]
    pub const fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }
}

/// A 2D floating-point vector.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Vector {
    pub x: f32,
    pub y: f32,
}

impl Vector {
    #[must_use]
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    #[must_use]
    pub fn length(self) -> f32 {
        self.x.hypot(self.y)
    }
}

/// An axis-aligned rectangle in unsigned pixel coordinates.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl Rect {
    #[must_use]
    pub const fn new(x: u32, y: u32, width: u32, height: u32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// Returns the half-open intersection with another rectangle, or None when empty.
    #[must_use]
    pub fn intersection(self, other: Self) -> Option<Self> {
        let left = self.x.max(other.x);
        let top = self.y.max(other.y);
        let right = (u64::from(self.x) + u64::from(self.width))
            .min(u64::from(other.x) + u64::from(other.width));
        let bottom = (u64::from(self.y) + u64::from(self.height))
            .min(u64::from(other.y) + u64::from(other.height));
        if u64::from(left) >= right || u64::from(top) >= bottom {
            return None;
        }
        Some(Self::new(left, top, (right - u64::from(left)) as u32, (bottom - u64::from(top)) as u32))
    }

    #[must_use]
    pub fn contains(self, point: Point) -> bool {
        if point.x < 0 || point.y < 0 {
            return false;
        }

        let x = point.x as u32;
        let y = point.y as u32;
        x >= self.x
            && y >= self.y
            && x < self.x.saturating_add(self.width)
            && y < self.y.saturating_add(self.height)
    }
}

#[cfg(test)]
mod tests {
    use super::{Point, Rect, Vector};

    #[test]
    fn rectangle_intersection_handles_overlap_touching_and_empty_rectangles() {
        assert_eq!(Rect::new(2, 3, 8, 7).intersection(Rect::new(6, 1, 5, 5)), Some(Rect::new(6, 3, 4, 3)));
        assert_eq!(Rect::new(0, 0, 4, 4).intersection(Rect::new(4, 0, 2, 2)), None);
        assert_eq!(Rect::new(0, 0, 0, 4).intersection(Rect::new(0, 0, 2, 2)), None);
    }

    #[test]
    fn vector_length_is_euclidean() {
        assert_eq!(Vector::new(3.0, 4.0).length(), 5.0);
    }

    #[test]
    fn rectangle_contains_only_points_inside_its_half_open_bounds() {
        let rect = Rect::new(10, 20, 5, 4);
        assert!(rect.contains(Point::new(10, 20)));
        assert!(rect.contains(Point::new(14, 23)));
        assert!(!rect.contains(Point::new(15, 23)));
        assert!(!rect.contains(Point::new(14, 24)));
        assert!(!rect.contains(Point::new(-1, 20)));
    }
}
