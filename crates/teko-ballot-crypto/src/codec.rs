//! Canonical encodings. Points are SEC1-compressed (33 bytes) and the identity is
//! refused; scalars are 32 bytes big-endian and must be < the group order.

use p256::elliptic_curve::ff::PrimeField;
use p256::elliptic_curve::group::GroupEncoding;
use p256::{AffinePoint, ProjectivePoint, Scalar};

pub const POINT_LEN: usize = 33;
pub const SCALAR_LEN: usize = 32;

pub fn point_to_bytes(p: &ProjectivePoint) -> [u8; POINT_LEN] {
    let mut out = [0u8; POINT_LEN];
    let repr = AffinePoint::from(*p).to_bytes();
    for (o, b) in out.iter_mut().zip(repr.iter()) {
        *o = *b;
    }
    out
}

/// `None` for malformed bytes, a point off the curve, or the identity.
pub fn point_from_bytes(b: &[u8]) -> Option<ProjectivePoint> {
    let arr: [u8; POINT_LEN] = b.try_into().ok()?;
    // SEC1 compressed only (tag 2 or 3): the decoder underneath is lenient about tags.
    if !matches!(arr.first(), Some(2 | 3)) {
        return None;
    }
    let a = Option::<AffinePoint>::from(AffinePoint::from_bytes(&arr.into()))?;
    let p = ProjectivePoint::from(a);
    // Canonical: it must re-encode to exactly the same bytes, and never be the identity.
    (p != ProjectivePoint::IDENTITY && point_to_bytes(&p) == arr).then_some(p)
}

pub fn scalar_to_bytes(s: &Scalar) -> [u8; SCALAR_LEN] {
    let mut out = [0u8; SCALAR_LEN];
    let repr = s.to_repr();
    for (o, b) in out.iter_mut().zip(repr.iter()) {
        *o = *b;
    }
    out
}

/// `None` unless canonical (< group order).
pub fn scalar_from_bytes(b: &[u8]) -> Option<Scalar> {
    let arr: [u8; SCALAR_LEN] = b.try_into().ok()?;
    Option::<Scalar>::from(Scalar::from_repr(arr.into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_canonical_points_and_scalars_decode() {
        let g = ProjectivePoint::GENERATOR;
        assert_eq!(point_from_bytes(&point_to_bytes(&g)), Some(g));
        assert!(point_from_bytes(&[5u8; 33]).is_none(), "bad SEC1 prefix");
        assert!(point_from_bytes(&[0u8; 33]).is_none());
        assert!(
            point_from_bytes(&point_to_bytes(&g)[1..]).is_none(),
            "short"
        );
        let mut off = point_to_bytes(&g);
        off[1] ^= 0xFF; // almost surely not a valid x
        let _ = point_from_bytes(&off); // must not panic either way
        assert!(scalar_from_bytes(&[0xFF; 32]).is_none(), ">= group order");
        assert!(scalar_from_bytes(&[1u8; 31]).is_none());
        let s = Scalar::from(7u64);
        assert_eq!(scalar_from_bytes(&scalar_to_bytes(&s)), Some(s));
    }
}
