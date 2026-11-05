//! 2D affine transformations using column vectors and row-major storage.
//!
//! A point is transformed as x' = m11*x + m12*y + tx,
//! y' = m21*x + m22*y + ty. Composition follows matrix multiplication:
//! `a.then(b)` applies `a` first, then `b`.

use crate::Vector;

/// A 2D affine transformation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Transform2D {
    pub m11: f32,
    pub m12: f32,
    pub m21: f32,
    pub m22: f32,
    pub tx: f32,
    pub ty: f32,
}

impl Default for Transform2D {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Transform2D {
    pub const IDENTITY: Self = Self {
        m11: 1.0,
        m12: 0.0,
        m21: 0.0,
        m22: 1.0,
        tx: 0.0,
        ty: 0.0,
    };

    #[must_use]
    pub const fn new(m11: f32, m12: f32, m21: f32, m22: f32, tx: f32, ty: f32) -> Self {
        Self {
            m11,
            m12,
            m21,
            m22,
            tx,
            ty,
        }
    }

    #[must_use]
    pub const fn translation(x: f32, y: f32) -> Self {
        Self::new(1.0, 0.0, 0.0, 1.0, x, y)
    }

    #[must_use]
    pub const fn scale(x: f32, y: f32) -> Self {
        Self::new(x, 0.0, 0.0, y, 0.0, 0.0)
    }

    #[must_use]
    pub fn rotation(radians: f32) -> Self {
        let (sin, cos) = radians.sin_cos();
        Self::new(cos, -sin, sin, cos, 0.0, 0.0)
    }

    #[must_use]
    pub fn around(origin: Vector, transform: Self) -> Self {
        Self::translation(-origin.x, -origin.y)
            .then(transform)
            .then(Self::translation(origin.x, origin.y))
    }

    #[must_use]
    pub fn transform_point(self, point: Vector) -> Vector {
        Vector::new(
            self.m11 * point.x + self.m12 * point.y + self.tx,
            self.m21 * point.x + self.m22 * point.y + self.ty,
        )
    }

    #[must_use]
    pub fn transform_vector(self, vector: Vector) -> Vector {
        Vector::new(
            self.m11 * vector.x + self.m12 * vector.y,
            self.m21 * vector.x + self.m22 * vector.y,
        )
    }

    /// Returns the composition that applies self, then next.
    #[must_use]
    pub fn then(self, next: Self) -> Self {
        Self::new(
            next.m11 * self.m11 + next.m12 * self.m21,
            next.m11 * self.m12 + next.m12 * self.m22,
            next.m21 * self.m11 + next.m22 * self.m21,
            next.m21 * self.m12 + next.m22 * self.m22,
            next.m11 * self.tx + next.m12 * self.ty + next.tx,
            next.m21 * self.tx + next.m22 * self.ty + next.ty,
        )
    }

    #[must_use]
    pub fn determinant(self) -> f32 {
        self.m11 * self.m22 - self.m12 * self.m21
    }

    /// Returns None for singular or non-finite matrices.
    #[must_use]
    pub fn inverse(self) -> Option<Self> {
        let det = self.determinant();
        if !det.is_finite()
            || det.abs() <= f32::EPSILON
            || ![self.m11, self.m12, self.m21, self.m22, self.tx, self.ty]
                .iter()
                .all(|v| v.is_finite())
        {
            return None;
        }
        let inv = 1.0 / det;
        let a = self.m22 * inv;
        let b = -self.m12 * inv;
        let c = -self.m21 * inv;
        let d = self.m11 * inv;
        Some(Self::new(
            a,
            b,
            c,
            d,
            -(a * self.tx + b * self.ty),
            -(c * self.tx + d * self.ty),
        ))
    }

    #[must_use]
    pub fn is_finite(self) -> bool {
        [self.m11, self.m12, self.m21, self.m22, self.tx, self.ty]
            .iter()
            .all(|v| v.is_finite())
    }
}

#[cfg(test)]
mod tests {
    use super::Transform2D;
    use crate::Vector;

    fn close(a: f32, b: f32) {
        assert!((a - b).abs() < 0.0001, "{a} != {b}");
    }

    #[test]
    fn identity_preserves_points_and_vectors() {
        let p = Vector::new(3.0, -4.0);
        assert_eq!(Transform2D::IDENTITY.transform_point(p), p);
        assert_eq!(Transform2D::IDENTITY.transform_vector(p), p);
    }

    #[test]
    fn translation_does_not_affect_vectors() {
        let t = Transform2D::translation(5.0, -2.0);
        assert_eq!(
            t.transform_point(Vector::new(1.0, 2.0)),
            Vector::new(6.0, 0.0)
        );
        assert_eq!(
            t.transform_vector(Vector::new(1.0, 2.0)),
            Vector::new(1.0, 2.0)
        );
    }

    #[test]
    fn rotation_and_non_uniform_scale_work() {
        let p = Transform2D::rotation(core::f32::consts::FRAC_PI_2)
            .transform_point(Vector::new(2.0, 0.0));
        close(p.x, 0.0);
        close(p.y, 2.0);
        assert_eq!(
            Transform2D::scale(2.0, 3.0).transform_point(Vector::new(2.0, 2.0)),
            Vector::new(4.0, 6.0)
        );
    }

    #[test]
    fn composition_applies_in_documented_order() {
        let t = Transform2D::translation(2.0, 0.0).then(Transform2D::scale(3.0, 3.0));
        assert_eq!(
            t.transform_point(Vector::new(1.0, 0.0)),
            Vector::new(9.0, 0.0)
        );
    }

    #[test]
    fn inverse_round_trips_points() {
        let t = Transform2D::translation(7.0, -3.0)
            .then(Transform2D::rotation(0.7))
            .then(Transform2D::scale(2.0, 0.5));
        let p = Vector::new(12.0, -8.0);
        let restored = t.inverse().unwrap().transform_point(t.transform_point(p));
        close(restored.x, p.x);
        close(restored.y, p.y);
    }

    #[test]
    fn singular_and_non_finite_transforms_have_no_inverse() {
        assert!(Transform2D::scale(0.0, 1.0).inverse().is_none());
        assert!(Transform2D::new(f32::NAN, 0.0, 0.0, 1.0, 0.0, 0.0)
            .inverse()
            .is_none());
    }

    #[test]
    fn around_keeps_the_origin_fixed() {
        let origin = Vector::new(4.0, 6.0);
        let t = Transform2D::around(origin, Transform2D::scale(2.0, 2.0));
        let result = t.transform_point(origin);
        close(result.x, origin.x);
        close(result.y, origin.y);
    }
}
