use std::ops::Add;

use plonky2_field::{ops::Square, types::Field};

use crate::curve::curve_types::{AffinePoint, Curve, ProjectivePoint};

impl<C: Curve> Add<ProjectivePoint<C>> for ProjectivePoint<C> {
    type Output = ProjectivePoint<C>;

    // https://www.hyperelliptic.org/EFD/g1p/auto-twisted-projective.html
    fn add(self, rhs: ProjectivePoint<C>) -> Self::Output {
        let ProjectivePoint {
            x: x1,
            y: y1,
            z: z1,
        } = self;
        let ProjectivePoint {
            x: x2,
            y: y2,
            z: z2,
        } = rhs;

        debug_assert!(z1.is_nonzero());
        debug_assert!(z2.is_nonzero());

        let a = z1 * z2;
        let b = a.square();
        let c = x1 * x2;
        let d = y1 * y2;
        let e = C::D * (c * d);
        let f = b - e;
        let g = b + e;
        let x3 = a * f * ((x1 + y1) * (x2 + y2) - c - d);
        let y3 = a * g * (d - C::A * c);
        let z3 = f * g;

        ProjectivePoint::nonzero(x3, y3, z3)
    }
}

impl<C: Curve> Add<AffinePoint<C>> for ProjectivePoint<C> {
    type Output = ProjectivePoint<C>;

    fn add(self, rhs: AffinePoint<C>) -> Self::Output {
        self + rhs.to_projective()
    }
}

impl<C: Curve> Add<AffinePoint<C>> for AffinePoint<C> {
    type Output = AffinePoint<C>;

    fn add(self, rhs: AffinePoint<C>) -> Self::Output {
        let AffinePoint { x: x1, y: y1 } = self;
        let AffinePoint { x: x2, y: y2 } = rhs;

        let x1x2 = x1 * x2;
        let y1y2 = y1 * y2;
        let x1y2 = x1 * y2;
        let y1x2 = y1 * x2;

        let dx1x2y1y2 = C::D * x1x2 * y1y2;

        let x3 = (x1y2 + y1x2) / (C::BaseField::ONE + dx1x2y1y2);
        let y3 = (y1y2 - C::A * x1x2) / (C::BaseField::ONE - dx1x2y1y2);

        Self { x: x3, y: y3 }
    }
}
