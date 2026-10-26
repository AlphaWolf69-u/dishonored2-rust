//! Compact entity-frame matrix operations, distinct from Havok padded matrices.
use super::{basis_product, ReferenceFrame};
use crate::float::{add, divide, multiply, subtract};

/// Full 10ee70. A rejected small determinant leaves the matrix untouched.
/// Cofactors are single precision; reciprocal and final products use doubles.
pub fn invert_basis(basis: &mut [[f32; 3]; 3]) -> bool {
    let [[a, b, c], [d, e, f], [g, h, i]] = *basis;
    let cofactors = [
        subtract(multiply(e, i), multiply(f, h)),
        subtract(multiply(g, f), multiply(d, i)),
        subtract(multiply(d, h), multiply(g, e)),
    ];
    let determinant = add(
        add(multiply(a, cofactors[0]), multiply(b, cofactors[1])),
        multiply(c, cofactors[2]),
    );
    let absolute = f32::from_bits(determinant.to_bits() & 0x7fff_ffff);
    // COMISS/JBE admits unordered determinants as well as >= threshold.
    if absolute < f32::from_bits(0x2834_24dc) {
        return false;
    }
    let inverse = 1.0f64 / f64::from(determinant);
    let result = [
        [
            cofactors[0],
            subtract(multiply(c, h), multiply(b, i)),
            subtract(multiply(b, f), multiply(c, e)),
        ],
        [
            cofactors[1],
            subtract(multiply(a, i), multiply(c, g)),
            subtract(multiply(c, d), multiply(a, f)),
        ],
        [
            cofactors[2],
            subtract(multiply(b, g), multiply(a, h)),
            subtract(multiply(a, e), multiply(b, d)),
        ],
    ];
    *basis = result.map(|row| row.map(|v| (f64::from(v) * inverse) as f32));
    true
}

/// Full 110080's compact-matrix quaternion, with its actual diagonal branches
/// and sign convention. The physics body's matrix conversion is a different rule.
pub fn basis_quaternion(basis: [[f32; 3]; 3]) -> [f32; 4] {
    let [[a, b, c], [d, e, f], [g, h, i]] = basis;
    // COMISS/JBE includes unordered comparisons in these three branches.
    let (mut length, result) = if !(i < 0.) {
        let difference = subtract(d, b);
        if !(f32::from_bits(e.to_bits() ^ 0x8000_0000) > a) {
            let length = add(add(i, 1.), add(e, a));
            (length, [subtract(h, f), subtract(c, g), difference, length])
        } else {
            let length = subtract(add(i, 1.), add(e, a));
            (length, [add(g, c), add(h, f), length, difference])
        }
    } else {
        let sum = add(d, b);
        if !(a > e) {
            let length = subtract(subtract(1., i), subtract(a, e));
            (length, [sum, length, add(h, f), subtract(c, g)])
        } else {
            let length = add(subtract(1., i), subtract(a, e));
            (length, [length, sum, add(g, c), subtract(h, f)])
        }
    };
    if !(length > f32::MIN_POSITIVE) {
        length = f32::MIN_POSITIVE;
    }
    let scale = multiply(divide(1., length.sqrt()), 0.5);
    result.map(|v| multiply(scale, v))
}

/// 753c10's retained local-binding update after movement. It deliberately
/// continues with the unchanged parent basis if 10ee70 rejects inversion.
pub fn relative_binding(parent: ReferenceFrame, child: ReferenceFrame) -> ([f32; 3], [f32; 4]) {
    let mut inverse = parent.basis;
    invert_basis(&mut inverse);
    let displacement =
        std::array::from_fn::<_, 3, _>(|i| subtract(child.position[i], parent.position[i]));
    let offset = std::array::from_fn(|i| {
        add(
            add(
                multiply(inverse[1][i], displacement[1]),
                multiply(inverse[0][i], displacement[0]),
            ),
            multiply(inverse[2][i], displacement[2]),
        )
    });
    (
        offset,
        basis_quaternion(basis_product(child.basis, inverse)),
    )
}

#[cfg(test)]
#[path = "basis_tests.rs"]
mod tests;
