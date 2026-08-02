//! Points, rectangles and 2×3 affine matrices in PDF's row-vector convention.

/// A point in PDF user space (bottom-left origin, y-up) unless a function says otherwise.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

impl Point {
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

/// An axis-aligned rectangle with `x0 <= x1` and `y0 <= y1`.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
}

impl Rect {
    pub fn from_corners(ax: f32, ay: f32, bx: f32, by: f32) -> Self {
        Self {
            x0: ax.min(bx),
            y0: ay.min(by),
            x1: ax.max(bx),
            y1: ay.max(by),
        }
    }

    pub const EMPTY: Rect = Rect {
        x0: 0.0,
        y0: 0.0,
        x1: 0.0,
        y1: 0.0,
    };

    pub fn width(&self) -> f32 {
        self.x1 - self.x0
    }

    pub fn height(&self) -> f32 {
        self.y1 - self.y0
    }

    pub fn is_empty(&self) -> bool {
        self.width() <= 0.0 || self.height() <= 0.0
    }

    pub fn union(&self, other: &Rect) -> Rect {
        if self.is_empty() && other.is_empty() {
            return *self;
        }
        if self.is_empty() {
            return *other;
        }
        if other.is_empty() {
            return *self;
        }
        Rect {
            x0: self.x0.min(other.x0),
            y0: self.y0.min(other.y0),
            x1: self.x1.max(other.x1),
            y1: self.y1.max(other.y1),
        }
    }

    pub fn intersect(&self, other: &Rect) -> Rect {
        Rect {
            x0: self.x0.max(other.x0),
            y0: self.y0.max(other.y0),
            x1: self.x1.min(other.x1),
            y1: self.y1.min(other.y1),
        }
    }
}

/// An affine transform `[a b c d e f]`, applied to row vectors: `(x, y, 1) · M`.
///
/// This is the convention the PDF specification uses for the CTM and text matrices, and the
/// convention pdfium-render exposes, so composition reads the same way in both: transforming by
/// `A` then `B` is `A.concat(B)`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Matrix {
    pub a: f32,
    pub b: f32,
    pub c: f32,
    pub d: f32,
    pub e: f32,
    pub f: f32,
}

impl Default for Matrix {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Matrix {
    pub const IDENTITY: Matrix = Matrix {
        a: 1.0,
        b: 0.0,
        c: 0.0,
        d: 1.0,
        e: 0.0,
        f: 0.0,
    };

    pub const fn new(a: f32, b: f32, c: f32, d: f32, e: f32, f: f32) -> Self {
        Self { a, b, c, d, e, f }
    }

    pub fn translate(tx: f32, ty: f32) -> Self {
        Self::new(1.0, 0.0, 0.0, 1.0, tx, ty)
    }

    pub fn scale(sx: f32, sy: f32) -> Self {
        Self::new(sx, 0.0, 0.0, sy, 0.0, 0.0)
    }

    /// `self` then `other`: the matrix that applies `self` first.
    pub fn concat(&self, other: &Matrix) -> Matrix {
        Matrix {
            a: self.a * other.a + self.b * other.c,
            b: self.a * other.b + self.b * other.d,
            c: self.c * other.a + self.d * other.c,
            d: self.c * other.b + self.d * other.d,
            e: self.e * other.a + self.f * other.c + other.e,
            f: self.e * other.b + self.f * other.d + other.f,
        }
    }

    pub fn apply(&self, p: Point) -> Point {
        Point {
            x: self.a * p.x + self.c * p.y + self.e,
            y: self.b * p.x + self.d * p.y + self.f,
        }
    }

    /// Transforms only the vector part (no translation).
    pub fn apply_vector(&self, x: f32, y: f32) -> (f32, f32) {
        (self.a * x + self.c * y, self.b * x + self.d * y)
    }

    /// The axis-aligned bounding box of the four transformed corners.
    pub fn apply_rect(&self, r: &Rect) -> Rect {
        let p0 = self.apply(Point::new(r.x0, r.y0));
        let p1 = self.apply(Point::new(r.x1, r.y0));
        let p2 = self.apply(Point::new(r.x0, r.y1));
        let p3 = self.apply(Point::new(r.x1, r.y1));
        Rect {
            x0: p0.x.min(p1.x).min(p2.x).min(p3.x),
            y0: p0.y.min(p1.y).min(p2.y).min(p3.y),
            x1: p0.x.max(p1.x).max(p2.x).max(p3.x),
            y1: p0.y.max(p1.y).max(p2.y).max(p3.y),
        }
    }

    /// Rotation, in radians, of the transformed x-axis.
    pub fn rotation(&self) -> f32 {
        self.b.atan2(self.a)
    }

    /// Length of the transformed unit y vector: the factor a font size is scaled by.
    pub fn y_scale(&self) -> f32 {
        (self.c * self.c + self.d * self.d).sqrt()
    }

    pub fn x_scale(&self) -> f32 {
        (self.a * self.a + self.b * self.b).sqrt()
    }

    pub fn invert(&self) -> Option<Matrix> {
        let det = self.a * self.d - self.b * self.c;
        if det.abs() < 1e-12 {
            return None;
        }
        let inv = 1.0 / det;
        Some(Matrix {
            a: self.d * inv,
            b: -self.b * inv,
            c: -self.c * inv,
            d: self.a * inv,
            e: (self.c * self.f - self.d * self.e) * inv,
            f: (self.b * self.e - self.a * self.f) * inv,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn concat_applies_left_first() {
        let scale = Matrix::scale(2.0, 2.0);
        let shift = Matrix::translate(10.0, 0.0);
        let m = scale.concat(&shift);
        let p = m.apply(Point::new(1.0, 1.0));
        assert_eq!((p.x, p.y), (12.0, 2.0));
    }

    #[test]
    fn invert_round_trips() {
        let m = Matrix::new(2.0, 1.0, -1.0, 3.0, 5.0, -2.0);
        let inv = m.invert().unwrap();
        let p = inv.apply(m.apply(Point::new(3.0, 4.0)));
        assert!((p.x - 3.0).abs() < 1e-4 && (p.y - 4.0).abs() < 1e-4);
    }
}
