use std::{fmt::Debug, hash::Hash, ops::Neg};

use plonky2_field::{
    ops::Square,
    types::{Field, PrimeField},
};
use serde::{Deserialize, Serialize};

// To avoid implementation conflicts from associated types,
// see https://github.com/rust-lang/rust/issues/20400
pub struct CurveScalar<C: Curve>(pub <C as Curve>::ScalarField);

/// A Twisted Edwards curve.
pub trait Curve: 'static + Sync + Sized + Copy + Debug {
    type BaseField: PrimeField;
    type ScalarField: PrimeField;

    const A: Self::BaseField;
    const D: Self::BaseField;

    const GENERATOR_AFFINE: AffinePoint<Self>;

    const GENERATOR_PROJECTIVE: ProjectivePoint<Self> = ProjectivePoint {
        x: Self::GENERATOR_AFFINE.x,
        y: Self::GENERATOR_AFFINE.y,
        z: Self::BaseField::ONE,
    };

    fn convert(x: Self::ScalarField) -> CurveScalar<Self> {
        CurveScalar(x)
    }

    fn assert_curve_valid() {
        assert!(Self::A.is_nonzero());
        assert!(Self::D.is_nonzero());
        assert_ne!(Self::A, Self::D);
    }
}

/// A point on a Twisted Edwards curve, represented in affine coordinates.
#[derive(Copy, Clone, Debug, Deserialize, Serialize, PartialEq, Eq, Hash)]
pub struct AffinePoint<C: Curve> {
    pub x: C::BaseField,
    pub y: C::BaseField,
}

impl<C: Curve> AffinePoint<C> {
    pub const ZERO: Self = Self {
        x: C::BaseField::ZERO,
        y: C::BaseField::ONE,
    };

    // TODO: Rename to new
    pub fn nonzero(x: C::BaseField, y: C::BaseField) -> Self {
        let point = Self { x, y };
        debug_assert!(point.is_valid());
        point
    }

    pub fn is_valid(&self) -> bool {
        let Self { x, y } = *self;
        C::A * x.square() + y.square() == C::BaseField::ONE + C::D * x.square() * y.square()
    }

    pub fn to_projective(&self) -> ProjectivePoint<C> {
        ProjectivePoint {
            x: self.x,
            y: self.y,
            z: C::BaseField::ONE,
        }
    }

    pub fn batch_to_projective(affine_points: &[Self]) -> Vec<ProjectivePoint<C>> {
        affine_points.iter().map(Self::to_projective).collect()
    }

    #[must_use]
    pub fn double(&self) -> Self {
        let AffinePoint { x, y, .. } = *self;

        let x_sq = x * x;
        let y_sq = y * y;

        let d_x_sq_y_sq = C::D * x_sq * y_sq;

        let x1 = (x * y).double() / (C::BaseField::ONE + d_x_sq_y_sq);
        let y1 = (y_sq - C::A * x_sq) / (C::BaseField::ONE - d_x_sq_y_sq);

        Self::nonzero(x1, y1)
    }
}

impl<C: Curve> Neg for AffinePoint<C> {
    type Output = AffinePoint<C>;

    fn neg(mut self) -> Self::Output {
        self.x = -self.x;
        self
    }
}

/// A point on a Twisted Edwards curve, represented in projective coordinates.
#[derive(Copy, Clone, Debug)]
pub struct ProjectivePoint<C: Curve> {
    pub x: C::BaseField,
    pub y: C::BaseField,
    pub z: C::BaseField,
}

impl<C: Curve> ProjectivePoint<C> {
    pub const ZERO: Self = Self {
        x: C::BaseField::ZERO,
        y: C::BaseField::ONE,
        z: C::BaseField::ONE,
    };

    pub fn nonzero(x: C::BaseField, y: C::BaseField, z: C::BaseField) -> Self {
        let point = Self { x, y, z };
        debug_assert!(point.is_valid());
        point
    }

    pub fn is_valid(&self) -> bool {
        let Self { x, y, z } = *self;

        let x_sq = x.square();
        let y_sq = y.square();
        let z_sq = z.square();

        z.is_nonzero() && (z_sq * (C::A * x_sq + y_sq) == z_sq.square() + C::D * x_sq * y_sq)
    }

    pub fn to_affine(&self) -> AffinePoint<C> {
        let Self { x, y, z } = *self;

        debug_assert!(z.is_nonzero());

        let z_inv = z.inverse();
        AffinePoint::nonzero(x * z_inv, y * z_inv)
    }

    pub fn batch_to_affine(proj_points: &[Self]) -> Vec<AffinePoint<C>> {
        let n = proj_points.len();
        let zs: Vec<C::BaseField> = proj_points.iter().map(|pp| pp.z).collect();
        let z_invs = C::BaseField::batch_multiplicative_inverse(&zs);

        let mut result = Vec::with_capacity(n);
        for i in 0..n {
            let Self { x, y, z } = proj_points[i];

            debug_assert!(z.is_nonzero());

            let z_inv = z_invs[i];
            let affine = AffinePoint::nonzero(x * z_inv, y * z_inv);
            result.push(affine);
        }

        result
    }

    // https://www.hyperelliptic.org/EFD/g1p/auto-twisted-projective.html
    #[must_use]
    pub fn double(&self) -> Self {
        let Self { x, y, z } = *self;

        debug_assert!(z.is_nonzero());

        let b = (x + y).square();
        let c = x.square();
        let d = y.square();
        let e = c * C::A;
        let f = e + d;
        let h = z.square();
        let j = f - C::BaseField::TWO * h;
        let x3 = (b - c - d) * j;
        let y3 = f * (e - d);
        let z3 = f * j;

        Self {
            x: x3,
            y: y3,
            z: z3,
        }
    }

    pub fn add_slices(a: &[Self], b: &[Self]) -> Vec<Self> {
        assert_eq!(a.len(), b.len());
        a.iter()
            .zip(b.iter())
            .map(|(&a_i, &b_i)| a_i + b_i)
            .collect()
    }

    #[must_use]
    pub fn neg(&self) -> Self {
        Self {
            x: -self.x,
            y: self.y,
            z: self.z,
        }
    }
}

impl<C: Curve> PartialEq for ProjectivePoint<C> {
    fn eq(&self, other: &Self) -> bool {
        let ProjectivePoint {
            x: x1,
            y: y1,
            z: z1,
        } = *self;
        let ProjectivePoint {
            x: x2,
            y: y2,
            z: z2,
        } = *other;

        debug_assert!(z1.is_nonzero());
        debug_assert!(z2.is_nonzero());

        // We want to compare (x1/z1, y1/z1) == (x2/z2, y2/z2).
        // But to avoid field division, it is better to compare (x1*z2, y1*z2) == (x2*z1, y2*z1).
        x1 * z2 == x2 * z1 && y1 * z2 == y2 * z1
    }
}

impl<C: Curve> Eq for ProjectivePoint<C> {}

impl<C: Curve> Neg for ProjectivePoint<C> {
    type Output = ProjectivePoint<C>;

    fn neg(self) -> Self::Output {
        let ProjectivePoint { x, y, z } = self;
        ProjectivePoint { x: -x, y, z }
    }
}

pub fn base_to_scalar<C: Curve>(x: C::BaseField) -> C::ScalarField {
    C::ScalarField::from_noncanonical_biguint(x.to_canonical_biguint())
}

pub fn scalar_to_base<C: Curve>(x: C::ScalarField) -> C::BaseField {
    C::BaseField::from_noncanonical_biguint(x.to_canonical_biguint())
}
